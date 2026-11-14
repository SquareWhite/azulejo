#!/bin/sh
# Emits one test escape sequence per case, each preceded by a visible marker line.
# Run inside a herdr pane (POSIX sh). Usage: passthrough.sh CASE [CASE ...]
# Pseudo-case "pause" sleeps 1 s. Mode cases are run as "X_on pause X_off" in one
# command so herdr's input encoding (e.g. bracketed paste around `pane run`) can't
# break the shell between cases.
ESC=$(printf '\033')
BEL=$(printf '\007')
ST="$ESC\\"

for c in "$@"; do
case "$c" in
  pause)       sleep 1 ;;
  truecolor)   printf '%s[38;2;255;100;0mTRUECOLOR%s[0m\n' "$ESC" "$ESC" ;;
  color256)    printf '%s[38;5;196mCOLOR256%s[0m\n' "$ESC" "$ESC" ;;
  undercurl)   printf '%s[4:3m%s[58:2::255:0:0mUNDERCURL%s[0m\n' "$ESC" "$ESC" "$ESC" ;;
  osc8)        printf '%s]8;;https://example.com/azj%sLINK%s]8;;%s\n' "$ESC" "$ST" "$ESC" "$ST" ;;
  osc52)       printf '%s]52;c;%s%s' "$ESC" "$(printf AZJCLIP | base64)" "$BEL"; echo OSC52-SENT ;;
  kittygfx)    printf '%s_Gf=24,s=1,v=1,a=T;AAAA%s' "$ESC" "$ST"; echo KGFX-SENT ;;
  osc9)        printf '%s]9;AZJ-NOTIFY-9%s' "$ESC" "$BEL"; echo OSC9-SENT ;;
  osc777)      printf '%s]777;notify;AZJ-T777;AZJ-B777%s' "$ESC" "$BEL"; echo OSC777-SENT ;;
  osc99)       printf '%s]99;;AZJ-NOTIFY-99%s' "$ESC" "$ST"; echo OSC99-SENT ;;
  bell)        printf '%s' "$BEL"; echo BELL-SENT ;;
  title)       printf '%s]2;AZJ-TITLE%s' "$ESC" "$BEL"; echo TITLE-SENT ;;
  osc7)        printf '%s]7;file://localhost/tmp/azj-osc7%s' "$ESC" "$BEL"; echo OSC7-SENT ;;
  bpaste_on)   printf '%s[?2004h' "$ESC"; echo BPASTE-ON ;;
  bpaste_off)  printf '%s[?2004l' "$ESC"; echo BPASTE-OFF ;;
  kkp_on)      printf '%s[>1u' "$ESC"; echo KKP-ON ;;
  kkp_off)     printf '%s[<u' "$ESC"; echo KKP-OFF ;;
  mouse_on)    printf '%s[?1000h%s[?1006h' "$ESC" "$ESC"; echo MOUSE-ON ;;
  mouse_off)   printf '%s[?1000l%s[?1006l' "$ESC" "$ESC"; echo MOUSE-OFF ;;
  cursor_bar)  printf '%s[5 q' "$ESC"; echo CURSOR-BAR ;;
  appcursor)   printf '%s[?1h' "$ESC"; echo DECCKM-ON ;;
  appcursor_off) printf '%s[?1l' "$ESC"; echo DECCKM-OFF ;;
  focus_on)    printf '%s[?1004h' "$ESC"; echo FOCUS-ON ;;
  focus_off)   printf '%s[?1004l' "$ESC"; echo FOCUS-OFF ;;
  *) echo "unknown case: $c" >&2; exit 2 ;;
esac
done
