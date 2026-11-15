#!/usr/bin/env python3
"""Stand-in for libghostty: herdr attach under a PTY, resize it, time the hops.

resize_client.py SESSION TERM_ID PANE_LOG MODE...   (run by resize-probe.sh)
  single N                              N isolated resizes, 300 ms apart
  burst INTERVAL_MS DURATION_MS REPS    resizes every INTERVAL for DURATION
  usr1 N                                pane redraws with no resize (SIGUSR1)
Times come from time.monotonic in both processes (one clock). A redraw is
seen at the client when resize_pane.py's token for that SIGWINCH shows up.
"""
import fcntl, os, pty, re, select, struct, sys, termios, time, statistics as st

sess, term, panelog, mode, *args = sys.argv[1:]
def setsz(fd, c, r): fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", r, c, 0, 0))
env = {k: v for k, v in os.environ.items() if not k.startswith("HERDR_")}
env["TERM"] = "xterm-256color"
pid, fd = pty.fork()
if pid == 0:
    os.execvpe("herdr", ["herdr", "--session", sess, "terminal", "attach", term, "--takeover"], env)
setsz(fd, 100, 30)
buf = b""
tokens = []  # (time, letter)
def pump(t):
    global buf
    end = time.monotonic() + t
    while True:
        left = end - time.monotonic()
        if left <= 0: return
        r, _, _ = select.select([fd], [], [], left)
        if r:
            try: d = os.read(fd, 1 << 20)
            except OSError: return
            now = time.monotonic()
            seen = len(buf)
            buf += d
            for m in re.finditer(rb"([A-Z])\1{7}", buf):
                if m.end() > seen: tokens.append((now, m.group(1).decode()))
            buf = buf[-16:]
pump(2.0)
def panelines():
    out = []
    for l in open(panelog).read().splitlines():
        n, t, c, r = l.split(); out.append((int(n), float(t), "%sx%s" % (c, r)))
    return out
def drawn_at(ev):
    n, tw, _ = ev; L = chr(65 + n % 26)
    for t, l in tokens:
        if l == L and t >= tw: return t
    return None

def pct(xs, p): xs = sorted(xs); return xs[min(len(xs) - 1, int(len(xs) * p))]
def summ(name, xs):
    if not xs: print(name, "n=0"); return
    print("%-38s n=%3d median=%6.1f p90=%6.1f min=%6.1f max=%6.1f" % (name, len(xs), st.median(xs), pct(xs, .9), min(xs), max(xs)))

if mode == "single":
    n = int(args[0]); winch, back, total = [], [], []
    for k in range(n):
        c, r = 100 + (k % 2) * 7, 30 + (k % 2) * 3
        tokens.clear(); w0 = len(panelines())
        t0 = time.monotonic(); setsz(fd, c, r)
        pump(0.3)
        pl = panelines()[w0:]
        if pl:
            winch.append((pl[0][1] - t0) * 1000)
            d = drawn_at(pl[0])
            if d: back.append((d - pl[0][1]) * 1000); total.append((d - t0) * 1000)
    summ("ioctl -> SIGWINCH in pane", winch)
    summ("pane write -> bytes at client", back)
    summ("ioctl -> redraw at client", total)
elif mode == "usr1":
    import signal as sg
    ppid = int(open(panelog + ".pid").read()); back = []
    for k in range(int(args[0])):
        tokens.clear(); w0 = len(panelines())
        os.kill(ppid, sg.SIGUSR1); pump(0.15)
        pl = panelines()[w0:]
        if pl and drawn_at(pl[0]): back.append((drawn_at(pl[0]) - pl[0][1]) * 1000)
    summ("no resize: pane write -> client", back)
elif mode == "burst":
    iv, dur, reps = float(args[0]) / 1000, float(args[1]) / 1000, int(args[2])
    lags, back, counts, winchs, lastlag = [], [], [], [], []
    for rep in range(reps):
        tokens.clear(); w0 = len(panelines())
        sent = []
        start = time.monotonic(); k = 0
        while time.monotonic() - start <= dur:
            c = 80 + k + rep % 2; r = 24 + k
            setsz(fd, c, r); sent.append((time.monotonic(), "%dx%d" % (c, r))); k += 1
            pump(max(0, start + k * iv - time.monotonic()))
        pump(0.4)
        pl = panelines()[w0:]
        counts.append(len(sent)); winchs.append(len(pl))
        req = {s: t for t, s in sent}
        for ev in pl:
            d = drawn_at(ev)
            if d and ev[2] in req:
                lags.append((d - req[ev[2]]) * 1000); back.append((d - ev[1]) * 1000)
        last = [ev for ev in pl if ev[2] == sent[-1][1]]
        if last and drawn_at(last[-1]): lastlag.append((drawn_at(last[-1]) - sent[-1][0]) * 1000)
    print("resizes sent per rep", counts, " SIGWINCH in pane", winchs)
    summ("request -> that size drawn at client", lags)
    summ("pane write -> bytes at client", back)
    summ("last request -> drawn at client", lastlag)
os.kill(pid, 9)
