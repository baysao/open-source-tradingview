#!/usr/bin/env bash
# Unattended PAUL plan loop: one fresh headless Claude session per plan.
#
# Each run starts a new session, so the context resets between plans. The SessionStart hook
# (~/.agents/paul-framework/scripts/session_reset_hook.py) sees .paul/session-reset.json, loads
# the CARL GLOBAL rules, runs /paul:resume and carries out the next action. The plan ends with
# /paul:pause, which writes a new marker; the loop then starts the next run.
#
# Usage: scripts/paul-loop.sh [max_plans]      (default 3)
# Env:   PAUL_LOOP_MODE  permission mode for each run (default auto; a mode that prompts
#                        fails headless, because nobody can answer)
#        PAUL_LOOP_CLAUDE  the claude binary (default claude; tests point it at a stub)
#
# Stops when: the cap is reached; a run exits non-zero; a run ends without a marker (it stopped
# at a decision, a human-verify checkpoint, an outward step, or the work is done); or the marker
# did not change (resume never consumed it, so another run would repeat the same work).
set -u

max="${1:-3}"
mode="${PAUL_LOOP_MODE:-auto}"
bin="${PAUL_LOOP_CLAUDE:-claude}"
root="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "paul-loop: not in a git repo" >&2; exit 2; }
cd "$root" || exit 2
marker=".paul/session-reset.json"
logdir=".paul/loop-logs"
mkdir -p "$logdir"

if [ ! -f "$marker" ]; then
  echo "paul-loop: no $marker. Run /paul:pause in a session first." >&2
  exit 1
fi

for n in $(seq 1 "$max"); do
  before="$(cat "$marker")"
  log="$logdir/$(date +%Y%m%d-%H%M%S)-run$n.log"
  echo "paul-loop: run $n/$max -> $log"
  echo "marker before: $before" > "$log"

  "$bin" -p "go: continue the PAUL session reset as the SessionStart context instructs" \
    --permission-mode "$mode" < /dev/null >> "$log" 2>&1
  rc=$?
  echo "exit: $rc" >> "$log"

  if [ "$rc" -ne 0 ]; then
    echo "paul-loop: run $n exited $rc. Stopping." >&2
    tail -n 20 "$log" >&2
    exit "$rc"
  fi
  if [ ! -f "$marker" ]; then
    echo "paul-loop: run $n ended without a new marker (checkpoint, decision, or done). Stopping."
    tail -n 20 "$log"
    exit 0
  fi
  if [ "$(cat "$marker")" = "$before" ]; then
    echo "paul-loop: marker unchanged after run $n (resume did not consume it). Stopping." >&2
    tail -n 20 "$log" >&2
    exit 3
  fi
  echo "paul-loop: run $n paused cleanly: $(cat "$marker")"
done

echo "paul-loop: cap of $max plans reached. Marker left for the next run or /paul:resume."
