// Spike 0.7: feed a captured herdr attach/observe byte stream into
// libghostty-vt and dump the resulting terminal state as text.
//
// Usage: vtdump <cols> <rows> < stream.bin
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ghostty/vt.h>

static bool write_out(void *ud, const uint8_t *data, size_t len) {
  return fwrite(data, 1, len, (FILE *)ud) == len;
}

#define CHECK(expr) do { GhosttyResult r_ = (expr); if (r_ != GHOSTTY_SUCCESS) { \
  fprintf(stderr, "%s failed: %d\n", #expr, (int)r_); return 1; } } while (0)

int main(int argc, char **argv) {
  uint16_t cols = argc > 1 ? (uint16_t)atoi(argv[1]) : 120;
  uint16_t rows = argc > 2 ? (uint16_t)atoi(argv[2]) : 40;

  GhosttyTerminal term;
  CHECK(ghostty_terminal_new(NULL, &term, cols, rows));

  uint8_t buf[65536];
  size_t n, total = 0;
  while ((n = fread(buf, 1, sizeof buf, stdin)) > 0) {
    ghostty_terminal_vt_write(term, buf, n);
    total += n;
  }

  uint16_t cx = 0, cy = 0;
  GhosttyTerminalScreen screen = GHOSTTY_TERMINAL_SCREEN_PRIMARY;
  size_t scrollback = 0;
  GhosttyString title = {0};
  CHECK(ghostty_terminal_get(term, GHOSTTY_TERMINAL_DATA_CURSOR_X, &cx));
  CHECK(ghostty_terminal_get(term, GHOSTTY_TERMINAL_DATA_CURSOR_Y, &cy));
  CHECK(ghostty_terminal_get(term, GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN, &screen));
  CHECK(ghostty_terminal_get(term, GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS, &scrollback));
  CHECK(ghostty_terminal_get(term, GHOSTTY_TERMINAL_DATA_TITLE, &title));

  printf("bytes=%zu grid=%ux%u cursor=(%u,%u) screen=%s scrollback_rows=%zu title=\"%.*s\"\n",
         total, cols, rows, cx, cy,
         screen == GHOSTTY_TERMINAL_SCREEN_ALTERNATE ? "alternate" : "primary",
         scrollback, (int)title.len, (const char *)title.ptr);

  GhosttyFormatterTerminalOptions opts = GHOSTTY_INIT_SIZED(GhosttyFormatterTerminalOptions);
  opts.emit = GHOSTTY_FORMATTER_FORMAT_PLAIN;
  opts.trim = true;
  GhosttyFormatter fmt;
  CHECK(ghostty_formatter_terminal_new(NULL, &fmt, term, opts));
  printf("----- screen (plain) -----\n");
  GhosttyWriter w = {.write = write_out, .userdata = stdout};
  CHECK(ghostty_formatter_format(fmt, w));
  printf("\n----- end -----\n");

  ghostty_formatter_free(fmt);
  ghostty_terminal_free(term);
  return 0;
}
