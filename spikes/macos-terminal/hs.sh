#!/usr/bin/env bash
# herdr against the azj-spike session only (never the user's live session).
unset HERDR_PANE_ID HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_ENV
export HERDR_SOCKET_PATH="$HOME/.config/herdr/sessions/azj-spike/herdr.sock"
exec herdr --session azj-spike "$@"
