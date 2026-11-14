# Time PTY resize -> SIGWINCH in a herdr pane, with no libghostty in the path.
import fcntl, os, pty, select, struct, subprocess, sys, termios, time
herdr, sess, term, log = sys.argv[1:5]
pid, fd = pty.fork()
if pid == 0:
    os.execv(herdr, [herdr, "--session", sess, "terminal", "attach", term, "--takeover"])
def setsz(r, c): fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", r, c, 0, 0))
setsz(30, 80)
def drain(t):
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.01)
        if r:
            try: os.read(fd, 65536)
            except OSError: return
drain(1.5)
lat = []
for k in range(20):
    n0 = len(open(log).read().splitlines())
    rows = 30 + (k % 2) * 6
    t0 = time.time(); setsz(rows, 80); os.kill(pid, 28)  # SIGWINCH to attach client, as a terminal would
    while True:
        drain(0.002)
        lines = open(log).read().splitlines()
        if len(lines) > n0:
            lat.append((float(lines[-1].split()[2]) - t0) * 1000); break
        if time.time() - t0 > 2: lat.append(None); break
    drain(0.25)
os.kill(pid, 9)
ok = sorted(x for x in lat if x is not None)
print("herdr-only resize->SIGWINCH ms: n=%d median=%.1f p95=%.1f min=%.1f max=%.1f" % (len(ok), ok[len(ok)//2], ok[int(len(ok)*.95)], ok[0], ok[-1]))
