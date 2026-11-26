#!/usr/bin/env bash
# check-mt5c.sh — the 08-03 contract suite (plan Task 2/3). Starts the fake MCP terminal, the
# Rust bridge, and the retired Python oracle (its copy lives at .paul/runs/0803/), all on
# the same fake with the same BRIDGE_TOKEN, then runs diff_bridge_contract.py over the four
# routes: one plain-JSON pass and one SSE pass. Exits with the differ's verdict.
#
# Patterns carried from check-0703.sh: ephemeral ports (L-029), PIDDIR + taskkill fallback
# (L-034), trap cleanup, SKIP_BUILD=1 escape hatch, self-terminating children (L-040), every
# judged exit status quoted bare (L-023), no judged pipes (R-8: pipefail is set).
# utf-8 pinned in the children (L-032). Not a debt check (L-030: it needs servers).
set -uo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"
RUN="$ROOT/.paul/runs/0803"
mkdir -p "$RUN"
FAKE_LOG="$RUN/fake.log"; RUST_LOG="$RUN/rust.log"; PY_LOG="$RUN/py.log"
FAKE_PID_FILE="$RUN/fake.pid"; RUST_PID_FILE="$RUN/rust.pid"; PY_PID_FILE="$RUN/py.pid"
CONTROL="$ROOT/.paul/runs/fake_mcp_control.json"
TOKEN="tok-mt5c-0803"

cleanup() {
  for f in "$FAKE_PID_FILE" "$RUST_PID_FILE" "$PY_PID_FILE"; do
    [ -f "$f" ] || continue
    pid=$(cat "$f")
    if kill -0 "$pid" 2>/dev/null; then
      kill -9 "$pid" 2>/dev/null || taskkill //F //PID "$pid" >/dev/null 2>&1
    fi
    rm -f "$f"
  done
}
trap cleanup EXIT
fail() { echo "FAIL: $1"; exit 1; }

pick_port() {
  python -c "import socket;s=socket.socket();s.bind(('127.0.0.1',0));print(s.getsockname()[1]);s.close()"
}

# --- build (L-029) ---
BIN="$ROOT/target/debug/mt5-bridge.exe"
if [ "${SKIP_BUILD:-0}" != "1" ]; then
  cargo build -p mt5-bridge || fail "mt5-bridge build failed"
fi
[ -f "$BIN" ] || fail "bridge binary missing at $BIN"

start_pass() {
  local label="$1"
  FAKE_PORT=$(pick_port); RUST_PORT=$(pick_port); PY_PORT=$(pick_port)
  [ -n "$FAKE_PORT" ] && [ -n "$RUST_PORT" ] && [ -n "$PY_PORT" ] || fail "port pick failed ($label)"

  echo '{}' > "$CONTROL"
  MT5_MCP_TOKEN=fake-tok-0803 python -I "$ROOT/.paul/runs/fake_mcp_terminal.py" --port "$FAKE_PORT" \
    >"$FAKE_LOG" 2>&1 &
  echo $! > "$FAKE_PID_FILE"
  for _ in $(seq 1 50); do
    grep -q "fake mcp terminal" "$FAKE_LOG" 2>/dev/null && break
    sleep 0.2
  done
  grep -q "fake mcp terminal" "$FAKE_LOG" || fail "fake did not come up (see $FAKE_LOG)"

  BRIDGE_TOKEN="$TOKEN" MT5_MCP_TOKEN=fake-tok-0803 \
    MT5_MCP_URL="http://127.0.0.1:$FAKE_PORT/mcp" BRIDGE_PORT="$RUST_PORT" \
    BRIDGE_STATE_DIR="$RUN" \
    "$BIN" >"$RUST_LOG" 2>&1 &
  echo $! > "$RUST_PID_FILE"

  BRIDGE_TOKEN="$TOKEN" MT5_MCP_TOKEN=fake-tok-0803 \
    MT5_MCP_URL="http://127.0.0.1:$FAKE_PORT/mcp" BRIDGE_PORT="$PY_PORT" \
    BRIDGE_STATE_DIR="$RUN" \
    python -I "$RUN/mt5_bridge_oracle.py" >"$PY_LOG" 2>&1 &
  echo $! > "$PY_PID_FILE"

  for _ in $(seq 1 50); do
    curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$RUST_PORT/health" >/dev/null 2>&1 \
      && curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PY_PORT/health" >/dev/null 2>&1 && break
    sleep 0.2
  done
  curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$RUST_PORT/health" >/dev/null 2>&1 \
    || fail "rust bridge did not come up (see $RUST_LOG)"
  curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PY_PORT/health" >/dev/null 2>&1 \
    || fail "python bridge did not come up (see $PY_LOG)"
  echo "== pass $label: fake :$FAKE_PORT, rust :$RUST_PORT, python :$PY_PORT =="
}

# --- pass 1: plain JSON ---
start_pass "plain"
python -I "$ROOT/.paul/runs/diff_bridge_contract.py" "$TOKEN" \
  "http://127.0.0.1:$RUST_PORT" "http://127.0.0.1:$PY_PORT"
DIFF1=$?
echo "pass plain differ exit=$DIFF1"
cleanup

# --- pass 2: SSE framing ---
start_pass "sse"
echo '{"sse": true}' > "$CONTROL"
python -I "$ROOT/.paul/runs/diff_bridge_contract.py" "$TOKEN" \
  "http://127.0.0.1:$RUST_PORT" "http://127.0.0.1:$PY_PORT"
DIFF2=$?
echo "pass sse differ exit=$DIFF2"
cleanup
echo '{}' > "$CONTROL"

# --- the ERRX error-parity case (both bridges must surface the fake's isError text) ---
start_pass "errors"
echo '{"logged_in": false}' > "$CONTROL"
H_RUST=$(curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$RUST_PORT/health")
H_PY=$(curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PY_PORT/health")
echo '{}' > "$CONTROL"
echo "logged-out health  rust: $H_RUST"
echo "logged-out health  python: $H_PY"
CANON_RUST=$(printf '%s' "$H_RUST" | python -c "import json,sys;print(json.dumps(json.load(sys.stdin),sort_keys=True,separators=(',',':')))")
CANON_PY=$(printf '%s' "$H_PY" | python -c "import json,sys;print(json.dumps(json.load(sys.stdin),sort_keys=True,separators=(',',':')))")
[ "$CANON_RUST" = "$CANON_PY" ] || fail "logged-out /health differs (canonical)"
echo "ERRX-CASE-OK logged-out /health identical"

if [ "$DIFF1" != "0" ] || [ "$DIFF2" != "0" ]; then
  echo "FAIL: contract diffs remain (plain=$DIFF1 sse=$DIFF2)"
  exit 1
fi
echo "MT5C-OK both passes identical, 0 differences"
exit 0
