/* A stand-in for omp (oh-my-pi) in mouse.sh, built and installed as `omp`
 * (and under other names) in a temp folder on the pane's PATH. A native
 * program, as omp is, so the pane's foreground process carries the name it
 * was started as (a script's would be its interpreter's). It turns on the
 * mouse modes omp does (1000 clicks, 1003 any motion, 1006 SGR), shows two
 * lines and appends every report it receives to the log file given as its
 * argument, one per line without the `ESC [ <` (`0;5;1M`). `q` quits. */
#include <stdio.h>
#include <string.h>
#include <termios.h>
#include <unistd.h>

static void put(const char *s) { (void)!write(STDOUT_FILENO, s, strlen(s)); }

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s LOG\n", argv[0]);
        return 2;
    }
    FILE *log = fopen(argv[1], "a");
    if (!log) return 1;
    setvbuf(log, NULL, _IOLBF, 0);
    struct termios saved, raw;
    tcgetattr(STDIN_FILENO, &saved);
    raw = saved;
    cfmakeraw(&raw);
    tcsetattr(STDIN_FILENO, TCSANOW, &raw);
    put("\033[2J\033[H" "omp alpha beta\r\n" "omp second line\r\n");
    put("\033[?1000h\033[?1003h\033[?1006h");
    char buf[4096];
    int quit = 0;
    while (!quit) {
        ssize_t n = read(STDIN_FILENO, buf, sizeof buf);
        if (n <= 0) break;
        for (ssize_t i = 0; i < n; i++) {
            if (buf[i] == 'q') {
                quit = 1;
                break;
            }
            if (buf[i] != '\033' || i + 2 >= n || buf[i + 1] != '[' || buf[i + 2] != '<') continue;
            ssize_t start = i + 3, j = start;
            while (j < n && ((buf[j] >= '0' && buf[j] <= '9') || buf[j] == ';')) j++;
            if (j < n && (buf[j] == 'M' || buf[j] == 'm')) {
                fprintf(log, "%.*s%c\n", (int)(j - start), buf + start, buf[j]);
                i = j;
            }
        }
    }
    put("\033[?1006l\033[?1003l\033[?1000l\033[2J\033[H");
    tcsetattr(STDIN_FILENO, TCSADRAIN, &saved);
    return 0;
}
