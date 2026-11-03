#!/usr/bin/env python3
"""PLAN.md step 0.2: re-runnable herdr probe. Called by scripts/herdr-probe.sh.

Starts an isolated named herdr session, answers the nine 0.2 questions with
measured evidence, prints a JSON line per check, then stops and deletes the
session. It never talks to the default session: every call goes through
`herdr --session <name>` with all inherited HERDR_* variables stripped.

Env:
  AZJ_PROBE_SESSION  session name (default azj-probe)
  AZJ_PROBE_OUT      directory for raw captures (default: a temp dir)
  AZJ_PROBE_TUI=1    also run the full TUI client next to a direct attach.
                     Off by default: the TUI starts connecting to your saved
                     SSH machines while it runs.
"""
import base64
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SESSION = os.environ.get("AZJ_PROBE_SESSION", "azj-probe")
OUT = os.environ.get("AZJ_PROBE_OUT") or tempfile.mkdtemp(prefix="azj-herdr-probe-")
os.makedirs(OUT, exist_ok=True)
ENV = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
ENV_S = dict(ENV, HERDR_SESSION=SESSION)
WS = "w1"  # replaced with the real workspace and root pane IDs at runtime
ROOT = "w1:p1"


def h(*args: str, check: bool = True) -> dict:
    r = subprocess.run(["herdr", "--session", SESSION, *args], capture_output=True, text=True, env=ENV)
    if check and r.returncode != 0:
        raise RuntimeError(f"herdr {' '.join(args)} failed: {r.stderr.strip()}")
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        return {"text": r.stdout, "stderr": r.stderr}


def report(question: str, **data) -> None:
    print(json.dumps({"q": question, **data}), flush=True)


def socket_path() -> str:
    out = subprocess.run(["herdr", "--session", SESSION, "status"], capture_output=True, text=True, env=ENV).stdout
    m = re.search(r"socket:\s*(\S+)", out.split("server:", 1)[-1])
    return m.group(1)


def rpc(method: str, params: dict) -> dict:
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(socket_path())
    s.sendall((json.dumps({"id": "probe", "method": method, "params": params}) + "\n").encode())
    buf = b""
    while not buf.endswith(b"\n"):
        buf += s.recv(65536)
    s.close()
    return json.loads(buf)


def pane(pid: str) -> dict:
    return h("pane", "get", pid)["result"]["pane"]


def read(pid: str, lines: int = 20) -> str:
    r = subprocess.run(["herdr", "--session", SESSION, "pane", "read", pid, "--source", "recent", "--lines", str(lines)],
                       capture_output=True, text=True, env=ENV)
    return r.stdout


def new_sh_tab(label: str, env: dict | None = None) -> str:
    args = ["tab", "create", "--workspace", WS, "--label", label, "--no-focus"]
    for k, v in (env or {}).items():
        args += ["--env", f"{k}={v}"]
    pid = h(*args)["result"]["root_pane"]["pane_id"]
    time.sleep(1.0)
    h("pane", "run", pid, "exec /bin/sh")
    time.sleep(0.5)
    h("pane", "run", pid, "PS1='$ '")
    time.sleep(0.3)
    return pid


def attach(term: str, seconds: float, name: str, *extra: str) -> tuple[dict, bytes]:
    out = os.path.join(OUT, name + ".bin")
    r = subprocess.run([sys.executable, os.path.join(HERE, "attach_pty.py"), term, str(seconds), out, *extra],
                       capture_output=True, text=True, env=ENV_S)
    return json.loads(r.stdout), open(out, "rb").read()


def attach_bg(term: str, seconds: float, name: str, *extra: str) -> subprocess.Popen:
    out = os.path.join(OUT, name + ".bin")
    return subprocess.Popen([sys.executable, os.path.join(HERE, "attach_pty.py"), term, str(seconds), out, *extra],
                            stdout=subprocess.PIPE, text=True, env=ENV_S)


def tail_msg(b: bytes) -> str:
    return b.rsplit(b"\x1b[0 q", 1)[-1].decode(errors="replace").strip()


def start_server() -> None:
    subprocess.Popen(["herdr", "--session", SESSION, "server"], stdout=open(os.path.join(OUT, "server.log"), "w"),
                     stderr=subprocess.STDOUT, env=ENV, start_new_session=True)
    for _ in range(40):
        if subprocess.run(["herdr", "--session", SESSION, "status"], capture_output=True, env=ENV).returncode == 0 \
                and "running" in subprocess.run(["herdr", "--session", SESSION, "status"], capture_output=True,
                                                text=True, env=ENV).stdout.split("server:", 1)[-1]:
            return
        time.sleep(0.25)
    raise SystemExit("probe server did not start")


def main() -> None:
    status = subprocess.run(["herdr", "--session", SESSION, "status"], capture_output=True, text=True, env=ENV).stdout
    if "status: running" in status.split("server:", 1)[-1]:
        raise SystemExit(f"session {SESSION} is already running; stop it first")
    start_server()
    report("version", status=re.sub(r"\s+", " ", subprocess.run(["herdr", "--version"], capture_output=True, text=True).stdout.strip()),
           out_dir=OUT)
    try:
        run_checks()
    finally:
        subprocess.run(["herdr", "--session", SESSION, "server", "stop"], capture_output=True, env=ENV)
        time.sleep(1.0)
        subprocess.run(["herdr", "session", "delete", SESSION], capture_output=True, env=ENV)


def first_pane() -> dict:
    # A fresh server may not have its first workspace yet; create one if needed.
    for _ in range(20):
        panes = h("pane", "list")["result"]["panes"]
        if panes:
            return panes[0]
        time.sleep(0.25)
    return h("workspace", "create", "--no-focus")["result"]["root_pane"]


def run_checks() -> None:
    global WS, ROOT
    # Q9: terminal ID from pane ID.
    root = first_pane()
    WS, ROOT = root["workspace_id"], root["pane_id"]
    report("9_terminal_id", pane_id=root["pane_id"], terminal_id=root.get("terminal_id"))

    # Q8: --env on tab create and pane split.
    p_env = h("tab", "create", "--workspace", WS, "--label", "env", "--no-focus",
              "--env", "AZULEJO_TILE_ID=tile_tab")["result"]["root_pane"]["pane_id"]
    p_split = h("pane", "split", "--pane", ROOT, "--direction", "down", "--no-focus",
                "--env", "AZULEJO_TILE_ID=tile_split")["result"]["pane"]["pane_id"]
    time.sleep(1.0)
    for p in (p_env, p_split):
        h("pane", "run", p, "echo AZJENV=$AZULEJO_TILE_ID TERM=$TERM TERM_PROGRAM=$TERM_PROGRAM")
    time.sleep(1.0)
    report("8_env", tab_create=re.findall(r"AZJENV=\S+.*", read(p_env))[-1:],
           pane_split=re.findall(r"AZJENV=\S+.*", read(p_split))[-1:])
    h("pane", "close", p_split)

    # Q4: event latency for pane created/closed.
    ev = subprocess.Popen([sys.executable, os.path.join(HERE, "events.py"), socket_path(), "6"],
                          stdout=subprocess.PIPE, text=True)
    time.sleep(0.8)
    ops = []
    for _ in range(5):
        t0 = time.time()
        pid = rpc("pane.split", {"target_pane_id": ROOT, "direction": "right", "focus": False})["result"]["pane"]["pane_id"]
        ops.append(("pane_created", pid, t0))
        time.sleep(0.3)
        t1 = time.time()
        rpc("pane.close", {"pane_id": pid})
        ops.append(("pane_closed", pid, t1))
        time.sleep(0.3)
    evs = [json.loads(l) for l in ev.communicate()[0].splitlines()]
    lat = []
    for kind, pid, t in ops:
        for e in evs:
            body = json.loads(e["event"])
            d = body.get("data", {})
            if body.get("event") == kind and (d.get("pane_id") == pid or d.get("pane", {}).get("pane_id") == pid):
                lat.append(round((e["t"] - t) * 1000, 1))
                break
    report("4_event_latency_ms", samples=lat, max=max(lat) if lat else None)

    # Q1 + Q5: escape sequence passthrough (observe stream) and resulting events.
    p_obs = new_sh_tab("passthrough")
    r = subprocess.run([sys.executable, os.path.join(HERE, "passthrough_probe.py"), p_obs, "observe"],
                       capture_output=True, text=True, env=ENV_S)
    report("1_observe_passthrough", results=[json.loads(l) for l in r.stdout.splitlines()])

    # Q1: what the direct-attach client writes to its host terminal.
    term = pane(p_obs)["terminal_id"]
    proc = attach_bg(term, 3.5, "attach_modes")
    time.sleep(1.2)
    h("pane", "run", p_obs, f"sh {HERE}/passthrough.sh bpaste_on mouse_on kkp_on appcursor focus_on osc52 osc9 "
                            "osc777 bell title kittygfx pause bpaste_off mouse_off kkp_off appcursor_off focus_off")
    proc.communicate()
    b = open(os.path.join(OUT, "attach_modes.bin"), "rb").read()
    needles = {"alt_screen_1049h": b"?1049h", "sync_output_2026": b"?2026h", "mouse_1000h": b"?1000h",
               "mouse_sgr_1006h": b"?1006h", "bracketed_paste_2004h": b"?2004h", "kitty_kbd_push": b"\x1b[>1u",
               "kitty_kbd_pop": b"\x1b[<1u", "decckm_1h": b"\x1b[?1h", "focus_1004h": b"?1004h",
               "osc52": b"]52;", "osc9": b"]9;", "osc777": b"]777;", "bel": b"\x07", "title_osc2": b"]2;",
               "kitty_graphics": b"\x1b_G", "osc8": b"]8;"}
    report("1_attach_host_bytes", present={k: (n in b) for k, n in needles.items()}, line_feeds=b.count(b"\n"))

    # Q7 (+ key passthrough): detach key and bytes the app receives.
    keys_file = os.path.join(OUT, "keys.txt")
    h("pane", "run", p_obs, f"printf '\\033[?1h'; stty raw -echo; dd bs=1 count=12 2>/dev/null | od -An -tx1 > {keys_file}; "
                            "stty sane; printf '\\033[?1l'")
    time.sleep(0.7)
    attach(term, 2.5, "keys", "--send-after", "0.6", "0278", "--send-after", "0.8", "0202", "--send-after", "1.0", "61",
           "--send-after", "1.2", "1b5b41", "--send-after", "1.4", "7a7a7a7a7a7a")
    time.sleep(0.5)
    received = open(keys_file).read().split() if os.path.exists(keys_file) else []
    summary, b = attach(term, 2.5, "detach", "--send-after", "1.0", "0271")
    report("7_detach_key", sent="02 78 | 02 02 | 61 | 1b 5b 41 (DECCKM on) | 7a x6", app_received=received,
           ctrl_b_q_exit_status=summary["client_exited_status"])

    # Q2: scrollback.
    h("pane", "run", p_obs, "clear; seq 1 300")
    time.sleep(1.0)
    wheel = "1b5b3c36343b31303b31304d" * 5
    proc = attach_bg(term, 2.5, "scroll", "--send-after", "1.0", wheel)
    time.sleep(1.8)
    during = pane(p_obs)["scroll"]
    proc.communicate()
    b = open(os.path.join(OUT, "scroll.bin"), "rb").read()
    report("2_scrollback", line_feeds_in_attach_output=b.count(b"\n"), scroll_after_5_wheel_notches=during)

    # Q3: two writers, takeover, observer alongside.
    a = attach_bg(term, 6, "writer_a", "--cols", "100", "--rows", "30")
    time.sleep(1.0)
    sb, bb = attach(term, 1.5, "writer_b", "--cols", "80", "--rows", "24")
    size_before = pane(p_obs)["scroll"]["viewport_rows"]
    obs = subprocess.run([sys.executable, os.path.join(HERE, "stream.py"), "observe", p_obs, "1.5",
                          os.path.join(OUT, "observer"), "--cols", "60", "--rows", "20"],
                         capture_output=True, text=True, env=ENV_S)
    frames = [json.loads(l) for l in open(os.path.join(OUT, "observer.ndjson"))]
    sc, _ = attach(term, 1.5, "writer_c", "--takeover", "--cols", "80", "--rows", "24")
    size_after = pane(p_obs)["scroll"]["viewport_rows"]
    sa = json.loads(a.communicate()[0])
    ba = open(os.path.join(OUT, "writer_a.bin"), "rb").read()
    report("3_two_writers",
           second_without_takeover={"exit": sb["client_exited_status"], "message": tail_msg(bb)},
           observer_alongside_writer={"frames": len(frames), "first_frame": {k: v for k, v in frames[0].items() if k != "bytes"} if frames else None},
           takeover={"exit_of_new": sc["client_exited_status"], "first_client_exit": sa["client_exited_status"],
                     "first_client_message": tail_msg(ba)},
           pane_rows_follow_controller={"before_takeover": size_before, "after_takeover": size_after})

    if os.environ.get("AZJ_PROBE_TUI") == "1":
        tui = attach_bg("tui", 6, "tui", "--cols", "140", "--rows", "45")
        time.sleep(1.5)
        rows_tui = pane(p_obs)["scroll"]["viewport_rows"]
        sd, _ = attach(term, 2.0, "with_tui", "--cols", "80", "--rows", "24")
        time.sleep(0.5)
        rows_after = pane(p_obs)["scroll"]["viewport_rows"]
        tui.communicate()
        report("3_tui_and_direct_attach", direct_attach_exit=sd["client_exited_status"],
               rows_with_tui_only=rows_tui, rows_after_attach_left=rows_after)

    # Q6: is layout.apply safe to mirror splits into an existing tab?
    created = rpc("tab.create", {"workspace_id": WS, "label": "layout", "focus": False})["result"]
    tab, p1 = created["tab"]["tab_id"], created["root_pane"]
    root = {"type": "split", "direction": "right", "ratio": 0.5,
            "first": {"type": "pane", "pane_id": p1["pane_id"]}, "second": {"type": "pane"}}
    applied = rpc("layout.apply", {"tab_id": tab, "root": root, "focus": False})
    lay = applied.get("result", {}).get("layout", {})
    panes = {p["pane_id"]: p["terminal_id"] for p in h("pane", "list", "--workspace", WS)["result"]["panes"]}
    report("6_layout_apply", original_pane=p1["pane_id"], original_terminal=p1["terminal_id"],
           original_still_exists=p1["pane_id"] in panes, result_tab=lay.get("tab_id"), requested_tab=tab,
           error=applied.get("error"))

    # Attach exit reasons: pane closed, server stopping.
    victim = h("pane", "split", "--pane", ROOT, "--direction", "down", "--no-focus")["result"]["pane"]
    proc = attach_bg(victim["terminal_id"], 2.5, "pane_closed")
    time.sleep(1.0)
    h("pane", "close", victim["pane_id"])
    s_closed = json.loads(proc.communicate()[0])
    proc = attach_bg(root_term := pane(ROOT)["terminal_id"], 2.5, "server_stop")
    time.sleep(1.0)
    subprocess.run(["herdr", "--session", SESSION, "server", "stop"], capture_output=True, env=ENV)
    s_stop = json.loads(proc.communicate()[0])
    report("attach_exit_reasons",
           pane_closed={"exit": s_closed["client_exited_status"], "message": tail_msg(open(os.path.join(OUT, "pane_closed.bin"), "rb").read())},
           server_stop={"exit": s_stop["client_exited_status"], "message": tail_msg(open(os.path.join(OUT, "server_stop.bin"), "rb").read())},
           root_terminal=root_term)


if __name__ == "__main__":
    main()
