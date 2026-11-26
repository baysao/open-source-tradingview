"""The 08-03 fake MCP terminal (check-mt5c.sh): a stdlib-only stand-in for the MT5 terminal's
built-in MCP server, so the Rust bridge and the Python bridge (bridge/mt5_bridge.py's contract
twin, retired after this phase) can be diffed against one deterministic oracle.

What it serves (POST /mcp only, JSON-RPC 2.0):
  initialize                -> result {} + an Mcp-Session-Id response header
  notifications/initialized -> 202-empty
  tools/call, five names:   get_time_information, get_chart_history,
                            get_chart_ticks_history, get_marketwatch_symbols,
                            get_trading_account_info

Deterministic data (no randomness, so both bridges see identical answers):
  - grid bars: one bar per `sec` ending at the window's last full slot, close = base + step;
    both bridges' span arithmetic is exercised, not mocked away
  - fixed tick list (one tick, whole seconds)
  - symbol rows: majors row, an unknown-category row (path "Other"), a heartbeat row
  - account: logged in, server "FAKE-SRV"

Test hooks:
  - symbol ERRX answers isError with fixed text on every tool
  - control file .paul/runs/fake_mcp_control.json flips between passes:
      {"logged_in": false}          -> get_trading_account_info answers balance-only
      {"sse": true}                 -> every body is SSE-framed ("data: " prefix)
      {"drop_session": true}        -> every request answers 440 (session gone); the client
                                       must reopen (initialize) and proceed
  - without MT5_MCP_TOKEN in the environment the fake refuses to start (the contract's guard)

utf-8 pinned (L-032); self-terminating (L-040): a lifetime timer ends the serve_forever loop,
so a trapped child can never outlive the suite. Port comes from argv (L-029's acquirer).
"""

import json
import os
import sys
import threading
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LIFETIME_S = 600
TF_SEC = {"M1": 60, "M5": 300, "M15": 900, "M30": 1800, "H1": 3600, "H4": 14400, "D1": 86400}
CATEGORY_PATH = {"majors": "Forex", "minors": "Forex", "exotic": "Forex", "crypto": "Crypto",
                 "metals": "Metals", "indices": "Indices", "energies": "Energies",
                 "stocks": "Stocks"}
CONTROL_FILE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fake_mcp_control.json")
ERRX_TEXT = "symbol ERRX does not exist"

STATE = {"offset_s": 0, "offset_src": "default (not yet measured)", "measured_at": 0}

# A fixed "now" for the grid so both bridges resolve the same newest bar no matter when the
# suite runs. 2025-11-26T09:40:00Z, a multiple of every TF's sec except H4/D1 multiples are
# fine too (both divide 86400's grid).
NOW = 1_764_150_000
SERVER_TIME_OFFSET = 0  # the fake's own clock is UTC; the terminal's offset is the bridges' job


def control():
    try:
        with open(CONTROL_FILE, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def iso(epoch):
    return datetime.fromtimestamp(epoch, timezone.utc).replace(tzinfo=None).isoformat(
        timespec="seconds")


def parse_time(s):
    return int(datetime.fromisoformat(s[:19]).replace(tzinfo=timezone.utc).timestamp())


def grid_bars(args):
    sym = args.get("symbol", "")
    tf = args.get("period", "M1")
    sec = TF_SEC.get(tf, 60)
    start = parse_time(args.get("datetime_from", iso(NOW - sec)))
    end = parse_time(args.get("datetime_to", iso(NOW)))
    last = min(end, NOW) // sec * sec
    # 6000 rows: above the bridges' 5000 clamp, so neither bridge's resync loop keeps widening
    # the span (the Python oracle's fromtimestamp raises OSError on pre-1970 windows).
    first = max(start // sec * sec, last - 6000 * sec)
    rows = []
    step = 0
    for t in range(first, last + 1, sec):
        step += 1
        close = 2000.0 + step * 0.5
        rows.append({
            "time": iso(t + SERVER_TIME_OFFSET),
            "open": close - 0.4,
            "high": close + 0.9,
            "low": close - 1.1,
            "close": close,
            "tick_volume": 100.0 + step,
        })
    return {"history": rows}


def ticks(_args):
    return {"history": [{"time_ms": iso(NOW), "bid": 2000.5, "ask": 2000.7}]}


def symbols(_args):
    return {"symbols": [
        {"symbol": "EURUSD", "description": "Euro vs US Dollar", "digits": 5,
         "category": "majors"},
        {"symbol": "WEIRD", "description": "Unknown category", "digits": 2,
         "category": "weird"},
        {"symbol": "HB", "description": "heartbeat", "digits": 0, "category": "heartbeat"},
    ]}


def account(_args):
    if not control().get("logged_in", True):
        return {"account": {"balance": 0, "login": 0}}
    return {"account": {"balance": 1000.0, "equity": 1000.0, "server": "FAKE-SRV",
                        "login": 123456}}


def time_info(_args):
    return {"trade_server_last_known_time": iso(NOW + SERVER_TIME_OFFSET),
            "utc_time": iso(NOW)}


TOOLS = {
    "get_time_information": time_info,
    "get_chart_history": grid_bars,
    "get_chart_ticks_history": ticks,
    "get_marketwatch_symbols": symbols,
    "get_trading_account_info": account,
}

SESSIONS = {}


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    timeout = 30

    def do_POST(self):
        if self.path != "/mcp":
            return self._reply(404, {"error": "not found"})
        token = os.environ.get("MT5_MCP_TOKEN", "").strip()
        if not token:
            return self._reply(500, {"error": "MT5_MCP_TOKEN is not set"})
        auth = self.headers.get("Authorization", "")
        if auth != "Bearer " + token:
            return self._reply(401, {"error": "unauthorized"})
        length = int(self.headers.get("Content-Length", 0))
        try:
            body = json.loads(self.rfile.read(length).decode("utf-8"))
        except ValueError:
            return self._reply(400, {"error": "bad json"})

        method = body.get("method", "")
        ctrl = control()
        if ctrl.get("drop_session"):
            # A stale session: the terminal restarted. The client must re-initialize.
            sid = self.headers.get("Mcp-Session-Id", "")
            if method == "initialize":
                pass  # handled below: this is the reopen itself
            else:
                return self._reply(440, {"error": "session expired"})

        if method == "initialize":
            new_sid = "fake-session-%d" % (len(SESSIONS) + 1)
            SESSIONS[new_sid] = True
            return self._reply(200, {"jsonrpc": "2.0", "id": body.get("id"),
                                     "result": {"protocolVersion": "2025-06-18"}},
                               session=new_sid)
        if method == "notifications/initialized":
            return self._reply(202, None)
        if method == "tools/call":
            name = body.get("params", {}).get("name", "")
            args = body.get("params", {}).get("arguments", {})
            if name == "ERRX" or args.get("symbol") == "ERRX":
                return self._reply(200, {"jsonrpc": "2.0", "id": body.get("id"), "result": {
                    "content": [{"type": "text", "text": ERRX_TEXT}], "isError": True}})
            tool = TOOLS.get(name)
            if tool is None:
                return self._reply(200, {"jsonrpc": "2.0", "id": body.get("id"), "result": {
                    "content": [{"type": "text", "text": "no tool %s" % name}],
                    "isError": True}})
            return self._reply(200, {"jsonrpc": "2.0", "id": body.get("id"), "result": {
                "content": [{"type": "text", "text": json.dumps(tool(args), separators=(",", ":"))}],
                "isError": False}})
        return self._reply(400, {"error": "unknown method"})

    def _reply(self, code, obj, session=None):
        if obj is None:
            body = b""
        elif control().get("sse"):
            text = json.dumps(obj, separators=(",", ":"))
            body = ("event: message\ndata: " + text + "\n\n").encode("utf-8")
        else:
            body = json.dumps(obj, separators=(",", ":")).encode("utf-8")
        self.send_response(code)
        if session:
            self.send_header("Mcp-Session-Id", session)
        if obj is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def log_message(self, *a):
        pass


def main():
    if not os.environ.get("MT5_MCP_TOKEN", "").strip():
        print("MT5_MCP_TOKEN is required", file=sys.stderr)
        sys.exit(1)
    port = int(sys.argv[sys.argv.index("--port") + 1]) if "--port" in sys.argv else 0
    server = ThreadingHTTPServer(("127.0.0.1", port), H)
    server.daemon_threads = True
    print("fake mcp terminal on 127.0.0.1:%d" % server.server_address[1], flush=True)
    threading.Timer(LIFETIME_S, server.shutdown).start()
    server.serve_forever()


if __name__ == "__main__":
    main()
