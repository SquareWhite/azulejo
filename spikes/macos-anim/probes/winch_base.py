# Baseline: PTY resize -> SIGWINCH logged by winch.sh with NO herdr in the path.
import fcntl, os, pty, select, struct, sys, termios, time
script, log = sys.argv[1:3]
pid, fd = pty.fork()
if pid == 0:
    os.execv("/usr/bin/env", ["env", "bash", script, "X", log])
def setsz(r, c): fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", r, c, 0, 0))
setsz(30, 80)
def drain(t):
    end = time.time() + t
    while time.time() < end:
        if select.select([fd], [], [], 0.01)[0]:
            try: os.read(fd, 65536)
            except OSError: return
drain(1.0)
lat = []
for k in range(20):
    n0 = len(open(log).read().splitlines())
    t0 = time.time(); setsz(30 + (k % 2) * 6, 80); os.kill(pid, 28)
    while True:
        drain(0.002)
        lines = open(log).read().splitlines()
        if len(lines) > n0: lat.append((float(lines[-1].split()[2]) - t0) * 1000); break
        if time.time() - t0 > 2: break
    drain(0.2)
os.kill(pid, 9)
ok = sorted(lat)
print("no-herdr resize->SIGWINCH-logged ms: n=%d median=%.1f p95=%.1f min=%.1f max=%.1f" % (len(ok), ok[len(ok)//2], ok[int(len(ok)*.95)], ok[0], ok[-1]))
