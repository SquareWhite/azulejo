#!/usr/bin/env python3
"""How herdr tells `done` from `idle`: its server-side "seen" state.
herdr-notes.md, question 11.

Starts a throwaway named session (default azj-test-probe-seen), prints one
JSON line per check, then stops and deletes it. Never talks to the default
session: every call pins the session's own socket, HERDR_* is stripped.

    python3 scripts/herdr-probe/seen.py
"""
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time

SESSION = os.environ.get("AZJ_PROBE_SESSION", "azj-test-probe-seen")
if not SESSION.startswith("azj-test-"):
    sys.exit("throwaway sessions only (azj-test-*)")
ENV = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}


def herdr(*args):
    r = subprocess.run(["herdr", "--session", SESSION, *args], capture_output=True, text=True, env=ENV)
    return r.stdout + r.stderr


def sock_path():
    m = re.search(r"socket:\s*(\S+)", herdr("status").split("server:", 1)[-1])
    path = m.group(1)
    assert f"/sessions/{SESSION}/" in path, path
    return path


def rpc(method, params):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(sock_path())
    s.sendall((json.dumps({"id": "probe", "method": method, "params": params}) + "\n").encode())
    buf = b""
    while not buf.endswith(b"\n"):
        chunk = s.recv(65536)
        if not chunk:
            break
        buf += chunk
    s.close()
    reply = json.loads(buf)
    return reply.get("result", reply.get("error"))


def report(q, **data):
    print(json.dumps({"q": q, **data}), flush=True)


def status(pane):
    return rpc("pane.get", {"pane_id": pane})["pane"]["agent_status"]


def agent(pane, state, seq):
    rpc("pane.report_agent", {"pane_id": pane, "source": "probe", "agent": "claude", "state": state, "seq": seq})
    time.sleep(0.2)


def focused():
    snap = rpc("session.snapshot", {})["snapshot"]
    ws = snap["focused_workspace_id"]
    tab = next(w["active_tab_id"] for w in snap["workspaces"] if w["workspace_id"] == ws)
    pane = next((p["pane_id"] for p in snap["panes"] if p["tab_id"] == tab and p.get("focused")), None)
    return {"workspace": ws, "tab": tab, "pane": pane}


def subscribe(panes, out):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(sock_path())
    subs = [{"type": "pane.agent_status_changed", "pane_id": p} for p in panes]
    subs += [{"type": "pane.updated"}]
    req = {"id": "sub", "method": "events.subscribe", "params": {"subscriptions": subs}}
    s.sendall((json.dumps(req) + "\n").encode())

    def run():
        buf = b""
        while True:
            try:
                chunk = s.recv(65536)
            except OSError:
                return
            if not chunk:
                return
            buf += chunk
            while b"\n" in buf:
                line, buf = buf.split(b"\n", 1)
                msg = json.loads(line)
                if "result" in msg:
                    continue
                ev = msg.get("event") or msg.get("type")
                data = msg.get("data", {})
                st = data.get("agent_status") or (data.get("pane") or {}).get("agent_status")
                out.append((ev, st))

    threading.Thread(target=run, daemon=True).start()
    return s


def start():
    if re.search(r"status:\s*running", herdr("status").split("server:", 1)[-1]):
        sys.exit(f"session {SESSION} is already running; stop it first")
    subprocess.Popen(["herdr", "--session", SESSION, "server"], stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, env=ENV, start_new_session=True)
    for _ in range(40):
        if re.search(r"status:\s*running", herdr("status").split("server:", 1)[-1]):
            return
        time.sleep(0.25)
    sys.exit("server did not start")


def stop():
    herdr("server", "stop")
    time.sleep(0.5)
    subprocess.run(["herdr", "session", "delete", SESSION], capture_output=True, env=ENV)


def checks():
    a = rpc("workspace.create", {"label": "a", "focus": True})["root_pane"]["pane_id"]
    b = rpc("workspace.create", {"label": "b", "focus": False})["root_pane"]["pane_id"]
    report("start", a=a, b=b, focused=focused())

    events = []
    sub = subscribe([a, b], events)
    time.sleep(0.3)

    # A background pane finishes: done, and stays done.
    agent(b, "working", 1)
    agent(b, "idle", 2)
    s1 = status(b)
    time.sleep(1.0)
    report("background_finish", status=s1, after_1s=status(b), events=list(events))
    events.clear()

    # Reading the pane doesn't mark it seen.
    rpc("pane.read", {"pane_id": b, "source": "visible"})
    report("read_does_not_mark_seen", status=status(b))

    # A pane that's herdr's focused one when it finishes.
    agent(a, "working", 1)
    agent(a, "idle", 2)
    report("focused_finish", status=status(a), focused=focused())
    events.clear()

    # pane.focus on the background pane: seen → idle; focus moves.
    before = focused()
    rpc("pane.focus", {"pane_id": b})
    time.sleep(0.3)
    report("pane_focus_marks_seen", status=status(b), focus_before=before, focus_after=focused(),
           events=list(events))

    # A new run that finishes again is unseen again.
    events.clear()
    agent(b, "working", 3)
    agent(b, "idle", 4)
    report("rerun_while_focused", status=status(b), focused=focused(), events=list(events))
    agent(a, "working", 3)
    agent(a, "idle", 4)
    report("rerun_background", status=status(a))

    # agent.focus does the same, by agent target.
    rpc("agent.focus", {"target": a})
    time.sleep(0.3)
    report("agent_focus_marks_seen", status=status(a), focused=focused())

    # Blocked isn't affected by focus.
    agent(b, "blocked", 5)
    rpc("pane.focus", {"pane_id": b})
    report("blocked_on_focus", status=status(b))

    # Focus on its workspace (not the pane)?
    agent(a, "working", 5)
    agent(a, "idle", 6)
    ws_a = rpc("pane.get", {"pane_id": a})["pane"]["workspace_id"]
    rpc("workspace.focus", {"workspace_id": ws_a})
    time.sleep(0.3)
    report("workspace_focus", status=status(a), focused=focused())
    sub.close()


def main():
    start()
    try:
        checks()
    finally:
        stop()


if __name__ == "__main__":
    main()
