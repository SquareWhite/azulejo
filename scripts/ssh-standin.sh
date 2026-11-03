#!/bin/sh
# A stand-in for `ssh` in tests (ADR 0011): takes the arguments Azulejo
# passes to ssh, ignores the options and the target, and runs the remote
# script on this machine. Lets tests drive the remote-machine code paths
# (herdr's remote-api-bridge, `ssh -tt … terminal attach`) against local,
# throwaway herdr sessions without any real SSH. Never use it outside tests.
#
# While a file named "<this script>.fail" exists it behaves like an
# unreachable host (exit 255). Tests copy the script to a temp dir and touch
# that file to cut the "SSH link". (Azulejo runs ssh with a cleaned
# environment, so an environment variable wouldn't get through.)
set -u
while [ $# -gt 0 ]; do
  case "$1" in
    -o|-F|-S|-i|-l|-p|-J) shift 2 ;;
    --) shift; break ;;
    -*) shift ;;
    *) shift ;; # the target
  esac
done
if [ -e "$0.fail" ]; then
  echo "ssh: connect to host standin port 22: Connection refused" >&2
  exit 255
fi
exec /bin/sh -c "$*"
