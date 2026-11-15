"""A mouse-aware program for mouse.sh: turns on mouse reporting (1000 +
SGR 1006), shows two lines, and appends every report it receives to the log
file given as its argument. `q` quits."""
import os, re, sys, termios, tty

log = open(sys.argv[1], "a", buffering=1)
fd = sys.stdin.fileno()
saved = termios.tcgetattr(fd)
tty.setraw(fd)
out = sys.stdout
out.write("\x1b[2J\x1b[H" "app alpha beta\r\n" "app link https://example.invalid/app\r\n")
out.write("\x1b[?1000h\x1b[?1006h")
out.flush()
try:
    while True:
        data = os.read(fd, 1024)
        if not data or b"q" in data:
            break
        for report in re.findall(rb"\x1b\[<[0-9;]+[Mm]", data):
            log.write(report[3:].decode() + "\n")
finally:
    out.write("\x1b[?1006l\x1b[?1000l\x1b[2J\x1b[H")
    out.flush()
    termios.tcsetattr(fd, termios.TCSADRAIN, saved)
