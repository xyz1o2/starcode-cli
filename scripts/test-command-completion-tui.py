#!/usr/bin/env python3
"""在独立网络命名空间及临时配置下验证真实 TUI：python3 -I <script> <binary>。"""

import hashlib
import json
import re
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]).resolve(strict=True)
root = Path(tempfile.mkdtemp(prefix="starcode-completion-probe-", dir="/tmp"))
socket = str(root / "tmux.sock")
with binary.open("rb") as executable:
    digest = hashlib.file_digest(executable, "sha256").hexdigest()
results = {"binary": str(binary), "sha256": digest, "passed": False, "cases": []}


def tmux(*args, check=True):
    return subprocess.run(
        ["tmux", "-S", socket, *args], text=True, capture_output=True, check=check
    ).stdout


def capture(session):
    return tmux("capture-pane", "-t", session, "-p")


def input_text(pane):
    lines = pane.splitlines()
    for index in range(len(lines) - 1, -1, -1):
        if re.search(r"─.*\d+:\d+", lines[index]):
            end = index + 1
            while end < len(lines) and not lines[end].startswith("─"):
                end += 1
            assert end < len(lines), pane
            return "\n".join(line.rstrip() for line in lines[index + 1:end])
    raise AssertionError("input border not found:\n" + pane)


def wait_for(test, session, label, timeout=40):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        pane = capture(session)
        if test(pane):
            return pane
        time.sleep(0.1)
    (root / (label + "-timeout.txt")).write_text(capture(session))
    raise RuntimeError("timeout waiting for " + label)


try:
    # 服务端及其 pane 子进程继承无路由、回环关闭的网络命名空间。
    network = subprocess.run(
        ["unshare", "--user", "--map-root-user", "--net", "sh", "-c",
         'ip -brief address; ip route; exec tmux -S "$1" -f /dev/null new-session -d -s anchor "exec /bin/sh"',
         "sh", socket],
        text=True, capture_output=True, check=True,
    )
    results["network_setup"] = network.stdout
    assert "lo" in network.stdout and "DOWN" in network.stdout
    for index, (text, key, pasted) in enumerate((
        ("/zzzzzzzzzz", "Tab", False), ("/zzzzzzzzzz", "Enter", False),
        ("/mcp zzzzzzzzzz", "Tab", False), ("/mcp zzzzzzzzzz", "Enter", False),
        ("/resum", "Tab", False), ("/resum", "Enter", False),
        ("/resum", "Tab", True),
    )):
        session = "case" + str(index)
        case_root = root / session
        case_root.mkdir()
        project = case_root / "project"
        home = case_root / "home"
        config = case_root / "config"
        for directory in (project, home, config):
            directory.mkdir()
        (home / ".star").mkdir()
        (home / ".star" / "user-settings.json").write_text(json.dumps({"uiLanguage": "en-US"}))
        (project / ".star").mkdir()
        (project / ".star" / "settings.json").write_text("{}")
        subprocess.run(["git", "init", "-q", str(project)], check=True)
        case = {"input": text, "key": key, "pasted": pasted, "passed": False}
        results["cases"].append(case)
        logs = root / (session + "-logs")
        logs.mkdir()
        argv = ["env", "-i", "PATH=/usr/bin:/bin", "TERM=screen-256color", "LANG=C.UTF-8",
                "HOME=" + str(home), "XDG_CONFIG_HOME=" + str(root / "xdg"),
                "STAR_CONFIG_DIR=" + str(config), "STAR_LOG_DIR=" + str(logs),
                "STAR_ANALYTICS_ENABLED=0", "STAR_TELEMETRY_ENABLED=0", "STAR_OPENAI_COMPATIBLE=1",
                str(binary), "--directory", str(project), "--api-key", "offline-fixture-only",
                "--base-url", "http://127.0.0.1:1/v1", "--model", "offline-fixture",
                "--permission-mode", "default"]
        tmux("new-session", "-d", "-s", session, "-x", "120", "-y", "40", "-c", str(project), shlex.join(argv))

        def ready(pane):
            log = logs / "agent.log"
            debug = logs / "starcode_debug.log"
            return (log.exists()
                    and "Initialization completed successfully" in log.read_text(errors="replace")
                    and debug.exists()
                    and "[UI] Event loop started" in debug.read_text(errors="replace")
                    and "Ask Star" in pane)

        wait_for(ready, session, session + "-ready")
        # 启动页绘制不等于键盘线程已经接管；先用无副作用字符确认输入通路就绪。
        tmux("send-keys", "-t", session, "-l", "q")
        wait_for(lambda pane: input_text(pane) == "q", session, session + "-keyboard-ready")
        tmux("send-keys", "-t", session, "BSpace")
        wait_for(lambda pane: "Ask Star" in pane, session, session + "-empty")
        if pasted:
            tmux("set-buffer", "-b", session, text)
            tmux("paste-buffer", "-p", "-b", session, "-t", session)
        else:
            tmux("send-keys", "-t", session, "-l", text)
        before = wait_for(lambda pane: text in pane, session, session + "-input")
        (root / (session + "-before.txt")).write_text(before)
        assert input_text(before) == text, before
        assert "> " + text not in before, before
        if text != "/resum":
            assert before.count(text) == 1 and "/mcp add" not in before, before
        time.sleep(0.8 if pasted else 0.3)  # 粘贴用例必须越过 350ms 提示保护期。
        tmux("send-keys", "-t", session, key)
        if text == "/resum" and not pasted:
            after = wait_for(lambda pane: input_text(pane).strip() == "/resume",
                             session, session + "-completed")
            (root / (session + "-after.txt")).write_text(after)
            assert "> /resume" not in after, after
            assert "Unknown command:" not in after, after
        elif key == "Tab":
            time.sleep(0.35)
            after = capture(session)
            (root / (session + "-after.txt")).write_text(after)
            assert input_text(after) == text, after
            assert "> " + text not in after, after
            assert "Unknown command:" not in after and "unrecognized subcommand" not in after, after
            # 随后的有序字符事件证明输入处理仍在工作；精确核对可发现 Tab 插入空白。
            tmux("send-keys", "-t", session, "-l", "q")
            synced = wait_for(lambda pane: text + "q" in pane, session, session + "-synced")
            (root / (session + "-synced.txt")).write_text(synced)
            assert input_text(synced) == text + "q", synced
        else:
            expected = "Unknown command: /zzzzzzzzzz" if text == "/zzzzzzzzzz" else "unrecognized subcommand"
            after = wait_for(lambda pane: expected in pane and "Type a message" in pane,
                             session, session + "-submitted")
            (root / (session + "-after.txt")).write_text(after)
            assert "> " + text in after, after
            if text.startswith("/mcp"):
                assert "zzzzzzzzzz" in after and "Usage: mcp" in after, after
        case["passed"] = True
        tmux("kill-session", "-t", session)
    results["passed"] = True
except Exception as error:
    results["error"] = str(error)
    raise
finally:
    tmux("kill-server", check=False)
    (root / "result.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
    print(json.dumps({"artifacts": str(root), **results}, ensure_ascii=False, indent=2))
