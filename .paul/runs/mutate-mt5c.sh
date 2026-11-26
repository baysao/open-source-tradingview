#!/usr/bin/env bash
# mutate-mt5c.sh — the 08-03 mutation driver (plan Task 3, L-003/L-009/L-013/R-10). Six
# mutations at the guards 08-ARCHITECTURE.md names; each is applied by exact-index replace,
# grepped as landed, run against its named guard (red expected, exit quoted bare), restored,
# grepped as gone, and the guard re-run (green expected). Ends with the full green set.
set -uo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"
FAILED=0

note() { printf '   %s\n' "$*"; }

mutate() { # file old new
  python -I - "$1" "$2" "$3" <<'PYEOF'
import sys
p, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p, encoding="utf-8").read()
n = s.count(old)
if n != 1:
    print(f"MUTATE-ANCHOR-FAIL: {n} occurrences in {p}")
    sys.exit(2)
open(p, "w", encoding="utf-8", newline="").write(s.replace(old, new))
PYEOF
}

run_mut() { # id file old new marker guard...
  local id="$1" file="$2" old="$3" new="$4" marker="$5"; shift 5
  cp "$file" "$file.bak"
  mutate "$file" "$old" "$new" || { echo "MUT $id: anchor failed"; FAILED=1; return; }
  if grep -qF -- "$marker" "$file"; then note "MUT $id landed ($marker)"; else
    echo "MUT $id: marker NOT found"; cp "$file.bak" "$file"; FAILED=1; return; fi
  local red=0
  "$@" || red=$?
  if [ "$red" != "0" ]; then note "MUT $id red exit=$red (expected non-zero)"; else
    echo "MUT $id: guard PASSED under mutation (expected FAIL)"; FAILED=1; fi
  mv -f "$file.bak" "$file"
  touch "$file"
  if grep -qF -- "$marker" "$file"; then echo "MUT $id: marker still present after restore"; FAILED=1; else
    note "MUT $id restored (marker gone)"; fi
  local green=0
  "$@" || green=$?
  if [ "$green" = "0" ]; then note "MUT $id green exit=0"; else
    echo "MUT $id: guard still red after restore (exit=$green)"; FAILED=1; fi
}

T=crates/mt5-bridge/src/term.rs
M=crates/mt5-bridge/src/mcp.rs
L=crates/mt5-bridge/src/live.rs
H=crates/mt5-bridge/src/http.rs

guard_crate() { cargo test -p mt5-bridge 2>&1 | tail -1; }

# --- the --list pin (L-026) before any mutation ---
COUNT=$(cargo test -p mt5-bridge -- --list 2>/dev/null | grep -c ": test")
[ "$COUNT" = "40" ] || { echo "L-026 FAIL: --list count $COUNT != 40"; exit 1; }
note "L-026: --list count 40 pinned"

# m1 offset-round: the refusal window widens so a stale clock passes
run_mut offset-round "$T" "if (d - h).abs() >= 90 {" "if (d - h).abs() >= 1800 {" \
  "abs() >= 1800" cargo test -p mt5-bridge term::tests::offset

# m2 session-reopen: the session id is never captured, so every call is a fresh handshake
run_mut session-reopen "$M" "            s.id = Some(id.to_string());" "            let _ = id;" \
  "let _ = id;" cargo test -p mt5-bridge mcp::tests

# m3 key-fields: volume drops out of the /wait key
run_mut key-fields "$T" '            py_str(b.volume)
        )' '            0.0_f64
        )' "0.0_f64" cargo test -p mt5-bridge term::tests::bar_key

# m4 limit-clamp: the ceiling drops below the contract's 5000
run_mut limit-clamp "$T" "    let limit = limit.clamp(1, 5000);" "    let limit = limit.clamp(1, 4000);" \
  "clamp(1, 4000)" cargo test -p mt5-bridge term::tests::bars_trims

# m5 auth-exact: the length check drops so a truncated header compares equal
run_mut auth-exact "$H" "    let same = got.len() == want.len() && bool::from(got.as_bytes().ct_eq(want.as_bytes()));" \
  "    let same = bool::from(got.as_bytes().ct_eq(want.as_bytes().get(..got.len()).unwrap_or(&[])));" \
  "get(..got.len())" cargo test -p mt5-bridge http::tests::a_missing_or_near_miss

# m6 wait-deadline: the waiter answers immediately without awaiting a change
run_mut wait-deadline "$L" "            let _ = tokio::time::timeout(left.min(StdDuration::from_secs(1)), rx.changed()).await;" \
  "            let snap = rx.borrow().clone(); return Ok(snap); // MUT-WD" \
  "MUT-WD" cargo test -p mt5-bridge live::tests

# --- final full green set ---
guard_crate
FULL=$?
if [ "$FULL" != "0" ]; then echo "FAIL: full crate suite red at the end (exit=$FULL)"; FAILED=1; fi
if [ "$FAILED" != "0" ]; then echo "MUTATIONS: FAILED"; exit 1; fi
echo "MUTATIONS-OK 6 red / 6 green, final suite green"
exit 0
