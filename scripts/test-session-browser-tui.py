#!/usr/bin/env python3
"""在无路由网络命名空间验证会话搜索：python3 -I <script> <binary>。"""

import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tempfile
import time

binary = Path(sys.argv[1]).resolve(strict=True)
root = Path(tempfile.mkdtemp(prefix="starcode-session-search-", dir="/tmp"))
socket = str(root / "tmux.sock")
with binary.open("rb") as executable:
    digest = hashlib.file_digest(executable, "sha256").hexdigest()
result = {"binary": str(binary), "sha256": digest, "passed": False}


def tmux(*args, check=True):
    return subprocess.run(
        ["tmux", "-S", socket, *args], text=True, capture_output=True, check=check
    ).stdout


def capture():
    return tmux("capture-pane", "-t", "probe", "-p")


def wait_for(predicate, label):
    deadline = time.monotonic() + 40
    while time.monotonic() < deadline:
        pane = capture()
        if predicate(pane):
            (root / (label + ".txt")).write_text(pane)
            return pane
        time.sleep(0.1)
    pane = capture()
    (root / (label + "-timeout.txt")).write_text(pane)
    raise AssertionError(label + " timed out:\n" + pane)


def selected(pane, title):
    return re.search(r"❯\s+" + re.escape(title) + r"\b", pane) is not None


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


def search_query(pane):
    match = re.search(r"^\s*Session Browser\s+/(.*)$", pane, re.MULTILINE)
    return match.group(1).rstrip() if match else None


try:
    network = subprocess.run(
        ["unshare", "--user", "--map-root-user", "--net", "sh", "-c",
         'ip -brief address; ip route; exec tmux -S "$1" -f /dev/null new-session -d -s anchor "exec /bin/sh"',
         "sh", socket], text=True, capture_output=True, check=True,
    )
    result["network_setup"] = network.stdout
    assert "lo" in network.stdout and "DOWN" in network.stdout
    project, home, config, logs = (root / name for name in ("project", "home", "config", "logs"))
    for directory in (project, home, config, logs):
        directory.mkdir()
    (home / ".star").mkdir()
    (home / ".star" / "user-settings.json").write_text(json.dumps({"uiLanguage": "en-US"}))
    sessions = project / ".star" / "sessions"
    sessions.mkdir(parents=True)
    (project / ".star" / "settings.json").write_text("{}")
    # 空正文使用会话文件名作为标题；不引入真实历史或可恢复的 agent 上下文。
    for identifier, created in (("other", 400), ("jk-first", 300), ("jk-second", 200)):
        (sessions / (identifier + ".json")).write_text(json.dumps({
            "id": identifier, "created_at": created, "chat_history": [],
        }))
    subprocess.run(["git", "init", "-q", str(project)], check=True)
    argv = ["env", "-i", "PATH=/usr/bin:/bin", "TERM=screen-256color", "LANG=C.UTF-8",
            "HOME=" + str(home), "XDG_CONFIG_HOME=" + str(root / "xdg"),
            "STAR_CONFIG_DIR=" + str(config), "STAR_LOG_DIR=" + str(logs),
            "STAR_ANALYTICS_ENABLED=0", "STAR_TELEMETRY_ENABLED=0", "STAR_OPENAI_COMPATIBLE=1",
            str(binary), "--directory", str(project), "--api-key", "offline-fixture-only",
            "--base-url", "http://127.0.0.1:1/v1", "--model", "offline-fixture",
            "--permission-mode", "default"]
    tmux("new-session", "-d", "-s", "probe", "-x", "120", "-y", "40", "-c", str(project), shlex.join(argv))

    def ready(pane):
        agent, debug = logs / "agent.log", logs / "starcode_debug.log"
        return (agent.exists() and "Initialization completed successfully" in agent.read_text(errors="replace")
                and debug.exists() and "[UI] Event loop started" in debug.read_text(errors="replace")
                and "Ask Star" in pane)

    wait_for(ready, "ready")
    draft = "untouched draft"
    tmux("send-keys", "-t", "probe", "-l", draft)
    wait_for(lambda p: input_text(p) == draft, "draft")
    tmux("send-keys", "-t", "probe", "C-p")
    wait_for(lambda p: "Filter actions" in p, "palette")
    tmux("send-keys", "-t", "probe", "-l", "Session browser")
    wait_for(lambda p: "Browse and resume past sessions" in p, "palette-filtered")
    tmux("send-keys", "-t", "probe", "Enter")
    wait_for(lambda p: "Session Browser" in p and "jk-first" in p and selected(p, "other"), "browser")
    tmux("send-keys", "-t", "probe", "-l", "j")
    wait_for(lambda p: " /j" in p and " /jk" not in p and selected(p, "jk-first")
             and "other" not in p and "jk-second" in p, "search-j")
    tmux("send-keys", "-t", "probe", "-l", "k")
    wait_for(lambda p: " /jk" in p and selected(p, "jk-first")
             and "other" not in p and "jk-second" in p, "search-jk")
    tmux("send-keys", "-t", "probe", "Down")
    wait_for(lambda p: " /jk" in p and selected(p, "jk-second"), "down")
    tmux("send-keys", "-t", "probe", "Up")
    wait_for(lambda p: " /jk" in p and selected(p, "jk-first"), "up")
    tmux("send-keys", "-t", "probe", "C-g", "M-x")
    # 用后续有序普通字符作为握手，避免仅凭旧截图断言快捷键已被处理。
    tmux("send-keys", "-t", "probe", "-l", "q")
    wait_for(lambda p: search_query(p) == "jkq", "shortcuts-ignored")
    tmux("send-keys", "-t", "probe", "BSpace")
    wait_for(lambda p: search_query(p) == "jk" and selected(p, "jk-first"), "query-restored")
    tmux("send-keys", "-t", "probe", "Escape")
    wait_for(lambda p: "Session Browser" not in p and input_text(p) == draft, "closed")
    tmux("send-keys", "-t", "probe", "-l", "q")
    wait_for(lambda p: input_text(p) == draft + "q" and "Session Browser" not in p, "draft-preserved")
    result["passed"] = True
except Exception as error:
    result["error"] = str(error)
    raise
finally:
    tmux("kill-server", check=False)
    (root / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2))
    print(json.dumps({"artifacts": str(root), **result}, ensure_ascii=False, indent=2))
