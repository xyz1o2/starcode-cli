#!/usr/bin/env python3
"""仅驱动指定 Rust 测试子进程，不启动模型或完整应用（Linux）。"""

import errno
import fcntl
import os
import pty
import select
import signal
import subprocess
import sys
import termios
import time

master, slave = pty.openpty()
before = termios.tcgetattr(slave)
output = bytearray()
child = None


def controlling_terminal():
    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


def receive_until(marker, timeout=10):
    deadline = time.monotonic() + timeout
    while marker not in output:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise AssertionError(f"PTY marker timeout: {marker!r}; output={output!r}")
        if select.select([master], [], [], min(remaining, 0.1))[0]:
            try:
                chunk = os.read(master, 65536)
            except OSError as error:
                if error.errno == errno.EIO:
                    chunk = b""
                else:
                    raise
            if not chunk:
                raise AssertionError(f"PTY closed before {marker!r}: {output!r}")
            output.extend(chunk)
        elif child.poll() is not None:
            raise AssertionError(f"child exited {child.returncode}: {output!r}")


try:
    child = subprocess.Popen(
        [sys.argv[1], "--exact", sys.argv[2], "--nocapture", "--test-threads=1"],
        stdin=slave, stdout=slave, stderr=slave, preexec_fn=controlling_terminal,
    )
    receive_until(b"EARLY_PTY_READY")
    # 先确认真实捕获线程消费普通键，再发送 raw 模式中的 Ctrl+C 字节。
    os.write(master, b"q")
    receive_until(b"EARLY_PTY_CAPTURED")
    if len(sys.argv) > 3 and sys.argv[3] == "paste":
        os.write(master, b"\x1b[200~" + "/help\n检查\x03".encode() + b"\x1b[201~")
        receive_until(b"EARLY_PTY_PASTED")
    os.write(master, b"\x03")
    receive_until(b"\x1b[?1049l")
    code = child.wait(timeout=10)
    while select.select([master], [], [], 0)[0]:
        try:
            chunk = os.read(master, 65536)
        except OSError as error:
            if error.errno == errno.EIO:
                break
            raise
        if not chunk:
            break
        output.extend(chunk)
    assert code == 130, f"expected exit 130, got {code}: {output!r}"
    assert termios.tcgetattr(slave) == before, f"terminal mode not restored: {output!r}"
    assert b"\x1b[?25h" in output, f"cursor not restored: {output!r}"
    assert b"panicked" not in output, f"capture thread panicked: {output!r}"
    print("PTY Ctrl+C: exit=130; termios restored; alternate screen exited; cursor shown")
finally:
    if child is not None and child.poll() is None:
        os.killpg(child.pid, signal.SIGKILL)
        child.wait()
    os.close(master)
    os.close(slave)
