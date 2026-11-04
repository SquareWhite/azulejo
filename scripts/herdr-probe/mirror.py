#!/usr/bin/env python3
"""ADR 0012: what herdr lets us do to a live tab's layout. herdr-notes.md, question 10.

Starts a throwaway named session (default azj-test-probe-mirror), prints one
JSON line per check, then stops and deletes it. Never talks to the default
session: every call pins the session's own socket, HERDR_* is stripped.

    python3 scripts/herdr-probe/mirror.py
"""
import json
import os
import re
import socket
import subprocess
import sys
import time

SESSION = os.environ.get("AZJ_PROBE_SESSION", "azj-test-probe-mirror")
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


def tree(node):
    if node["type"] == "pane":
        return node["pane_id"]
    return [node["direction"], node["ratio"], tree(node["first"]), tree(node["second"])]


def export(tab):
    layout = rpc("layout.export", {"tab_id": tab})["layout"]
    return {"zoomed": layout["zoomed"], "focused": layout["focused_pane_id"], "tree": tree(layout["root"])}


def report(q, **data):
    print(json.dumps({"q": q, **data}), flush=True)


def shell_pid(pane):
    return rpc("pane.process_info", {"pane_id": pane})["process_info"]["shell_pid"]


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
    time.sleep(1.0)
    subprocess.run(["herdr", "session", "delete", SESSION], capture_output=True, env=ENV)


def checks():
    ws = rpc("workspace.create", {"label": "probe", "focus": False})
    p1, tab = ws["root_pane"]["pane_id"], ws["root_pane"]["tab_id"]

    # Split with env, direction and ratio; the new pane is the second child.
    p2 = rpc("pane.split", {"target_pane_id": p1, "direction": "right", "ratio": 1 / 3,
                            "env": {"AZULEJO_TILE_ID": "probe"}, "focus": False})["pane"]["pane_id"]
    time.sleep(1.0)
    herdr("pane", "run", p2, "echo AZJ=$AZULEJO_TILE_ID")
    time.sleep(0.8)
    env_seen = "AZJ=probe" in herdr("pane", "read", p2, "--source", "recent", "--lines", "5")
    report("split", tree=export(tab)["tree"], env_seen=env_seen, note="ratio stored as f32")

    # Ratio by path: [] root, true = second child.
    p3 = rpc("pane.split", {"target_pane_id": p2, "direction": "down", "focus": False})["pane"]["pane_id"]
    rpc("layout.set_split_ratio", {"tab_id": tab, "path": [True], "ratio": 0.25})
    missing = rpc("layout.set_split_ratio", {"tab_id": tab, "path": [False], "ratio": 0.25})
    report("set_split_ratio", tree=export(tab)["tree"], bad_path=missing)

    # Swap keeps ratios with their slots.
    rpc("pane.swap", {"source_pane_id": p1, "target_pane_id": p3})
    report("swap", tree=export(tab)["tree"])

    # Zoom shows (and focuses) the pane; zooming another only moves focus.
    rpc("pane.zoom", {"pane_id": p2, "mode": "on"})
    first = export(tab)
    rpc("pane.zoom", {"pane_id": p3, "mode": "on"})
    report("zoom", on=first, other_pane=export(tab))

    # Moves: refused out of a zoomed tab and inside one tab.
    zoomed = rpc("pane.move", {"pane_id": p1, "destination": {"type": "new_tab"}})["move_result"]
    rpc("pane.zoom", {"pane_id": p3, "mode": "off"})
    same = rpc("pane.move", {"pane_id": p1, "destination": {"type": "tab", "tab_id": tab, "split": "right",
                                                           "target_pane_id": p2}})["move_result"]
    report("move_refusals", zoomed_tab=zoomed.get("reason"), same_tab=same.get("reason"))

    # A move between tabs keeps the process and terminal; the emptied tab goes.
    solo = rpc("tab.create", {"workspace_id": ws["workspace"]["workspace_id"], "focus": False})["root_pane"]
    time.sleep(1.0)
    pid, term = shell_pid(solo["pane_id"]), solo["terminal_id"]
    moved = rpc("pane.move", {"pane_id": solo["pane_id"], "destination": {
        "type": "tab", "tab_id": tab, "split": "down", "target_pane_id": p2, "ratio": 0.7}})["move_result"]
    tabs = [t["tab_id"] for t in rpc("session.snapshot", {})["snapshot"]["tabs"]]
    report("move_between_tabs", same_terminal=moved["pane"]["terminal_id"] == term,
           same_shell=shell_pid(moved["pane"]["pane_id"]) == pid, emptied_tab_gone=solo["tab_id"] not in tabs)

    # Another workspace: the pane ID changes, the terminal ID stays.
    other = rpc("workspace.create", {"label": "two", "focus": False})["root_pane"]
    across = rpc("pane.move", {"pane_id": p3, "destination": {
        "type": "tab", "tab_id": other["tab_id"], "split": "right", "target_pane_id": other["pane_id"]}})
    report("move_across_workspaces", before=p3, after=across["move_result"]["pane"]["pane_id"],
           terminal_kept=across["move_result"]["pane"]["terminal_id"] is not None)

    # layout.apply on a live tab still replaces its panes.
    t2 = other["tab_id"]
    before = {p["terminal_id"] for p in rpc("pane.list", {})["panes"] if p["tab_id"] == t2}
    root = rpc("layout.export", {"tab_id": t2})["layout"]["root"]
    applied = rpc("layout.apply", {"tab_id": t2, "root": root})
    after_tab = applied["layout"]["tab_id"]
    after = {p["terminal_id"] for p in rpc("pane.list", {})["panes"] if p["tab_id"] == after_tab}
    report("layout_apply", new_tab=after_tab != t2, terminals_replaced=not (before & after))

    # A restart keeps tree, ratios, zoom and pane IDs, with fresh terminals.
    rpc("pane.zoom", {"pane_id": p2, "mode": "on"})
    kept = export(tab)
    herdr("server", "stop")
    time.sleep(1.5)
    subprocess.Popen(["herdr", "--session", SESSION, "server"], stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, env=ENV, start_new_session=True)
    time.sleep(1.5)
    report("restart", same_layout=export(tab) == kept)


def main():
    start()
    try:
        checks()
    finally:
        stop()


if __name__ == "__main__":
    main()
