#!/usr/bin/env python3
"""离线验证真实历史搜索全文恢复：python3 -I <script> <binary>。"""

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
root = Path(tempfile.mkdtemp(prefix="starcode-history-search-", dir="/tmp"))
socket = str(root / "tmux.sock")
with binary.open("rb") as executable:
    digest = hashlib.file_digest(executable, "sha256").hexdigest()
result = {"binary": str(binary), "sha256": digest, "passed": False}


def tmux(*args, check=True):
    return subprocess.run(
        ["tmux", "-S", socket, *args], text=True, capture_output=True, check=check
    ).stdout


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


def wait_for(predicate, label):
    deadline = time.monotonic() + 40
    while time.monotonic() < deadline:
        pane = tmux("capture-pane", "-t", "probe", "-p")
        if predicate(pane):
            (root / (label + ".txt")).write_text(pane)
            return pane
        time.sleep(0.1)
    (root / (label + "-timeout.txt")).write_text(pane)
    raise AssertionError(label + " timed out:\n" + pane)


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
    original = "/help " + "检查长消息 🦀 " * 15 + "END-OF-ORIGINAL"
    (sessions / "fixture.json").write_text(json.dumps({
        "id": "fixture", "created_at": 300,
        "chat_history": [
            {"type": "User", "content": "fixture user", "timestamp": "2026-11-22T00:00:00Z"},
            {"type": "Assistant", "content": original, "timestamp": "2026-11-22T00:00:01Z"},
        ],
    }, ensure_ascii=False))
    subprocess.run(["git", "init", "-q", str(project)], check=True)
    argv = ["env", "-i", "PATH=/usr/bin:/bin", "TERM=screen-256color", "LANG=C.UTF-8",
            "HOME=" + str(home), "XDG_CONFIG_HOME=" + str(root / "xdg"),
            "STAR_CONFIG_DIR=" + str(config), "STAR_LOG_DIR=" + str(logs),
            "STAR_ANALYTICS_ENABLED=0", "STAR_TELEMETRY_ENABLED=0", "STAR_OPENAI_COMPATIBLE=1",
            str(binary), "--directory", str(project), "--api-key", "offline-fixture-only",
            "--base-url", "http://127.0.0.1:1/v1", "--model", "offline-fixture",
            "--permission-mode", "plan", "--resume", "fixture"]
    tmux("new-session", "-d", "-s", "probe", "-x", "300", "-y", "45", "-c", str(project), shlex.join(argv))

    def ready(pane):
        agent, debug = logs / "agent.log", logs / "starcode_debug.log"
        return (agent.exists() and "Initialization completed successfully" in agent.read_text(errors="replace")
                and debug.exists() and "[UI] Event loop started" in debug.read_text(errors="replace")
                and "Ask Star" in pane)

    wait_for(ready, "ready")
    tmux("send-keys", "-t", "probe", "-l", "draft")
    wait_for(lambda p: input_text(p) == "draft", "draft")
    tmux("send-keys", "-t", "probe", "C-r")
    wait_for(lambda p: "Search prompts" in p and "fixture user" in p, "history-open")
    # 查询完整内容中摘要之外的标记，避免错误选中用户条目。
    tmux("send-keys", "-t", "probe", "-l", "END-OF-ORIGINAL")
    wait_for(lambda p: "END-OF-ORIGINAL" in p and "Results (1" in p, "filtered")
    tmux("send-keys", "-t", "probe", "Enter")
    wait_for(lambda p: "Search prompts" not in p and input_text(p) == original, "full-text-restored")
    tmux("send-keys", "-t", "probe", "-l", "q")
    wait_for(lambda p: input_text(p) == original + "q", "still-editable")
    result["passed"] = True
except Exception as error:
    result["error"] = str(error)
    raise
finally:
    tmux("kill-server", check=False)
    (root / "result.json").write_text(json.dumps(result, ensure_ascii=False, indent=2))
    print(json.dumps({"artifacts": str(root), **result}, ensure_ascii=False, indent=2))
