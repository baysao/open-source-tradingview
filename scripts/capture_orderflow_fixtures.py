"""Order-flow golden fixtures for the Rust port (spec item 18; .paul/phases/02-orderflow/02-01-PLAN.md).

Subcommands:
  numeric   write tests/fixtures/orderflow/py_numeric.json from Python itself (no network)
  record    pass 1: download the inputs once (network) into tests/fixtures/orderflow/inputs/
  replay    pass 2: run hub/orderflow_api.py on the saved inputs with the network off and the clock
            frozen, and write cases/*.json and stores/*.json
  check     run `numeric` and `replay` into a temp dir and compare with the committed fixtures

Deleted in Phase 4 with the Python server.
"""
from __future__ import annotations

import json
import math
import os
import random
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIX = os.path.join(ROOT, "crates", "hub-core", "tests", "fixtures", "orderflow")


def need_312():
    if sys.version_info < (3, 12):
        sys.exit("needs Python 3.12 or newer: builtin sum() is only compensated (Neumaier) from 3.12")


def dump(path, obj):
    """Deterministic JSON: compact, dict order kept, floats by repr (they round-trip exactly)."""
    os.makedirs(os.path.dirname(path), exist_ok=True)
    text = json.dumps(obj, separators=(",", ":"), allow_nan=False, ensure_ascii=True) + "\n"
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


# ---------------------------------------------------------------- numeric tables

def _nice_step(x):  # copy of hub/orderflow_api.py nice_step, so this group needs no hub import
    if x <= 0:
        return 1.0
    e = 10 ** math.floor(math.log10(x))
    for m in (1, 2, 2.5, 5, 10):
        if x <= m * e:
            return m * e
    return 10 * e


def _err(fn, arg):
    try:
        v = fn(arg)
        return {"ok": v}
    except ValueError as e:
        return {"err": str(e)}


def numeric_tables():
    rnd = random.Random(20261123)
    rounds = []
    for _ in range(2500):
        mag = 10 ** rnd.uniform(-9, 8)
        x = rnd.choice((1, -1)) * rnd.random() * mag
        nd = rnd.choice((4, 6, 2, 0, 1, 3, 5))
        rounds.append([x, nd, round(x, nd)])
    for nd in range(0, 7):  # exact binary ties: (2m+1) / 2^(nd+1) has x * 10^nd ending in .5
        for _ in range(150):
            m = rnd.randrange(0, 2_000_000)
            x = rnd.choice((1, -1)) * (2 * m + 1) / 2 ** (nd + 1)
            rounds.append([x, nd, round(x, nd)])
    for x, nd in ((0.125, 2), (2.5, 0), (3.5, 0), (-0.00004, 4), (0.0, 4), (-0.0, 4), (1e-12, 6), (123456.78905, 4)):
        rounds.append([x, nd, round(x, nd)])

    sums = []
    for _ in range(600):
        n = rnd.randrange(1, 40)
        xs = [rnd.choice((1, -1)) * rnd.random() * 10 ** rnd.uniform(-6, 9) for _ in range(n)]
        sums.append([xs, sum(xs)])
    for xs in ([1e16, 1.0, -1e16], [0.1] * 10, [-0.0], [-0.0, -0.0], [1e308, 1e308, -1e308], [0.0]):
        try:
            sums.append([xs, sum(xs)])
        except OverflowError:
            pass
    sums = [s for s in sums if math.isfinite(s[1])]

    mods = []
    for _ in range(400):
        a = rnd.uniform(-1e6, 1e6)
        b = rnd.choice((rnd.uniform(0.001, 1000), -rnd.uniform(0.001, 1000)))
        mods.append([a, b, a % b])
    for a, b in ((-0.0, 5.0), (0.0, -5.0), (-3.0, 3.0), (3.0, -3.0), (5.5, 0.25), (-5.5, 0.25)):
        mods.append([a, b, a % b])

    maxmin = []
    for a, b in ((0.0, -0.0), (-0.0, 0.0), (1.0, 2.0), (2.0, 1.0), (-1.0, -1.0), (3.5, 3.5)):
        maxmin.append([a, b, max(a, b), min(a, b)])

    nice = []
    xs = [0.0, -1.0, 0.37, 7.0, 0.0071428, 3.0, 2.5, 12.5, 49.9, 50.0, 50.1, 99.99, 1e-7]
    for k in range(-22, 16):  # row sizes beyond 1e15 do not occur; Rust keeps ints in i64
        p = 10.0 ** k
        xs += [p, math.nextafter(p, 0.0), math.nextafter(p, math.inf), 2 * p, 2.5 * p, 5 * p, 3 * p, 7.3 * p]
    for _ in range(300):
        xs.append(10 ** rnd.uniform(-8, 8))
    for x in xs:
        v = _nice_step(x)
        nice.append([x, "int" if isinstance(v, int) else "float", v])

    ints = [_err(int, s) | {"s": s} for s in ("12", " 12 ", "-7", "+3", "1_000", "0", "abc", "1.5", "", " ", "1__0", "_1", "1_", "9" * 18, "'x'", 'a"b')]
    floats = [_err(float, s) | {"s": s} for s in ("1", "2.5", " 0.25 ", "-1e3", "1_0.5", "inf", "-Infinity", "x", "", "1..2", "70", "nan")]
    for r in floats:  # JSON has no inf/nan: keep them as strings
        if "ok" in r and not math.isfinite(r["ok"]):
            r["ok"] = repr(r["ok"])
    return {
        "python": sys.version.split()[0],
        "round": rounds,
        "sum": sums,
        "mod": mods,
        "maxmin": maxmin,
        "nice_step": nice,
        "int": ints,
        "float": floats,
    }


# ---------------------------------------------------------------- unit tables from the real helpers

FUTURES_UNITS = {"XAUUSDT", "BTCUSDT", "ETHUSDT", "COPPERUSDT", "NATGASUSDT", "XAGUSDT"}


def _hub():
    """Import the oracle modules from hub/ (their imports are lazy; no server start)."""
    sys.path.insert(0, os.path.join(ROOT, "hub"))
    import orderflow_api as ofa
    import orderflow_trades as ot
    return ofa, ot


def _rows_list(d):
    return [[t, list(v)] for t, v in d.items()]


def _call(fn, *a):
    try:
        return {"ok": fn(*a)}
    except Exception as e:  # the class and text are part of the contract
        return {"err": f"{e.__class__.__name__}: {e}"}


def unit_tables():
    ofa, ot = _hub()
    ofa.futures_symbols = lambda: FUTURES_UNITS
    tickers = ["RMT_XAUUSD", "frxXAUUSD", "BYBIT_XAUUSDT", "XAUUSD", "BTCUSD", "cryBTCUSD", "BYBIT_BTCUSDT",
               "BYBIT_S_ETHUSDT", "BYBIT_I_BTCUSDT", "BYBIT_XAUUSDT.MARK", "BYBIT_BTCUSDT.INDEX", "RMT_XCUUSD",
               "RMT_XNGUSD", "BINANCE:BTCUSDT", "XAUUSD247", "R_100", "frxEURUSD", "PAXGUSD", "xagusd", "SOLUSD"]
    real = [[t, ofa.real_symbol(t)] for t in tickers]

    sess = []
    base = 1795392000  # 2026-11-23 00:00 UTC, a Monday
    for d in range(-1, 9):
        for h in (0, 1, 6, 7, 12, 13, 21, 22, 23):
            t = base + d * 86400 + h * 3600 + 1234
            for mode in ("sessions", "day", "week", "other"):
                sess.append([t, mode, list(ofa.session_of(t, mode))])

    est = []
    def bars(ts, pattern):
        out = []
        for i, t in enumerate(ts):
            o, c = pattern[i % len(pattern)]
            out.append({"time": t, "open": o, "high": max(o, c) + 1.0, "low": min(o, c) - 1.0, "close": c, "volume": 1.5 + i})
        return out
    flat = [(10.0, 11.0), (11.0, 11.0), (11.0, 10.5), (10.5, 10.5), (10.5, 10.5), (10.5, 12.0), (12.0, 12.0)]
    for name, chunks, a, b in (
        ("one_chunk_flat_runs", [bars(list(range(6000, 6000 + 60 * 40, 60)), flat)], 6120, 7800),
        ("two_chunks_overlap", [bars(list(range(9000, 9000 + 60 * 60, 60)), flat), bars(list(range(5400, 5400 + 60 * 70, 60)), flat[::-1])], 6000, 12500),
        ("no_volume", [[{**x, "volume": 0.0} for x in bars(list(range(0, 60 * 10, 60)), flat)]], 0, 600),
    ):
        queue = list(chunks)
        calls = []
        def hist(ticker, itf, n, to_ms, queue=queue, calls=calls):
            calls.append([ticker, itf, n, to_ms])
            return queue.pop(0) if queue else []
        got = ofa._estimated_rows(hist, "X", "M1", a, b)
        est.append({"name": name, "chunks": chunks, "a": a, "b": b, "calls": calls,
                    "rows": _rows_list(got), "need_volume": _call(lambda: ofa._need_volume(got) and None)})

    shifts = []
    lower = {t: (1.0, 2.0, 0, 0.0, 0.0, 100.0 + (t % 7) * 0.1) for t in range(0, 60 * 80, 60)}
    chart = {t: (0.0, 0.0, 0, 0.0, 0.0, 100.25 + (t % 11) * 0.05) for t in range(0, 60 * 80, 300)}
    chart_neg0 = {0: (0.0, 0.0, 0, 0.0, 0.0, 100.0), 300: (0.0, 0.0, 0, 0.0, 0.0, 100.0)}
    lower_neg0 = {240: (0, 0, 0, 0, 0, 100.0), 540: (0, 0, 0, 0, 0, 100.0)}
    for name, lo, ch, sec, isec in (("m5_over_m1", lower, chart, 300, 60), ("no_gap", lower, {}, 300, 60),
                                    ("zero_gaps", lower_neg0, chart_neg0, 300, 60)):
        shifts.append({"name": name, "lower": _rows_list(lo), "chart": _rows_list(ch), "sec": sec, "isec": isec,
                       "out": [[t, g] for t, g in ofa._shift(lo, ch, sec, isec).items()]})

    hists = []
    rnd = random.Random(7)
    bars_a = [(rnd.random() * 5, rnd.random() * 5, 100 + rnd.random() * 3, 99 + rnd.random()) for _ in range(30)]
    bars_tie = [(1.0, 1.0, 101.0, 100.0), (1.0, 1.0, 103.0, 102.0), (0.5, 0.0, 100.5, 100.5)]
    for name, b, n_rows, row_size, va in (("auto_24", bars_a, 24, 0.0, 0.70), ("rowsize_0p25", bars_a, 24, 0.25, 0.68),
                                          ("tie_poc", bars_tie, 4, 0.0, 0.70), ("rows_zero", bars_a, 0, 0.0, 0.70),
                                          ("rows_negative", bars_a, -3, 0.0, 0.70), ("flat_bar", [(2.0, 1.0, 50.0, 50.0)], 24, 0.0, 0.70)):
        def run(b=b, n_rows=n_rows, row_size=row_size, va=va):
            lo, step, buy, sell = ofa._histogram(b, n_rows, row_size)
            return [lo, step, buy, sell, list(ofa._levels(lo, step, buy, sell, va))]
        hists.append({"name": name, "bars": [list(x) for x in b], "rows": n_rows, "row_size": row_size, "va": va, "result": _call(run)})

    ivs = [[(0, 10), (11, 20)], [(5, 9), (0, 3)], [(0, 10), (12, 20)], [], [(0, 100), (50, 60)]]
    unions = [[iv, ot._union(iv), [[a, b, ot._covers(iv, a, b)] for a, b in ((0, 20), (2, 8), (13, 15), (5, 4), (0, 0))]] for iv in ivs]

    rows_seq = []
    store = {}
    def fake_binance_rows(sym, itf, a, b):
        sec = ofa.TF_SEC[itf]
        return {t: (1.0, 0.5, 0, 2.0, 1.0, 1.5 + t / 1e6) for t in range(a - a % sec, b + 1, sec) if t >= a}
    saved = ofa._binance_rows
    ofa._binance_rows = lambda sym, itf, a, b: (store.setdefault("calls", []).append([a, b]), fake_binance_rows(sym, itf, a, b))[1]
    ofa._cache.clear()
    for a, b in ((60_000, 66_000), (63_000, 70_000), (50_000, 64_000), (40_000, 80_000), (55_000, 58_000)):
        store["calls"] = []
        got = ofa._rows("real", "BTCUSD", "M1", a, b, None)
        rows_seq.append({"a": a, "b": b, "fetched": store["calls"], "keys": list(got.keys())})
    ofa._binance_rows = saved
    ofa._cache.clear()
    return {"real_symbol": real, "futures": sorted(FUTURES_UNITS), "session_of": sess, "estimated": est,
            "shift": shifts, "histogram": hists, "union": unions, "rows_sequence": rows_seq}


def cmd_numeric(out_dir=FIX):
    dump(os.path.join(out_dir, "py_numeric.json"), numeric_tables())
    dump(os.path.join(out_dir, "py_units.json"), unit_tables())
    print("py_numeric.json and py_units.json written")


# ---------------------------------------------------------------- golden cases (record, replay)

import csv
import datetime as _dtm
import hashlib
import io as _io
import shutil
import socket
import tempfile
import types
import zipfile

INPUTS = os.path.join(FIX, "inputs")
D = _dtm.date(2026, 11, 17)                        # fixed past UTC Tuesday: the daily file exists
D0 = 1794873600                                    # D 00:00 UTC
NOW = D0 + 86400 + 12 * 3600                       # frozen clock: D+1 12:00 UTC (D is "yesterday")
H = 3600
EST_CANDIDATES = ["BYBIT_MNTUSDT", "BYBIT_FLOCKUSDT", "BYBIT_PLUMEUSDT", "BYBIT_ZEREBROUSDT"]


def case_specs(est_sym, est_row):
    """Each case: name, calls [(kind, query)], trade scenario per footprint call, day states."""
    xau = {"symbol": "frxXAUUSD", "feed": "deriv"}
    btc_bb = {"symbol": "BYBIT_BTCUSDT", "feed": "bybit"}
    btc = {"symbol": "BTCUSD", "feed": "binance"}
    est = {"symbol": est_sym, "feed": "bybit"}
    s = lambda d: str(d)
    return [
        ("delta_xau_deriv_m5_real", [("delta", {**xau, "tf": "M5", "from": s(D0 + 8 * H), "to": s(D0 + 14 * H)})]),
        ("delta_btc_bybit_m1_two_pages", [("delta", {**btc_bb, "tf": "M1", "from": s(D0 - 6 * H), "to": s(D0 + 23 * H)})]),
        ("delta_btc_binance_d1_unaligned", [("delta", {**btc, "tf": "D1", "from": s(D0 - 20 * 86400 + 5 * H), "to": s(D0)})]),
        ("delta_est_h4_paging", [("delta", {**est, "tf": "H4", "from": s(D0 - 25 * 86400), "to": s(D0)})]),
        ("delta_deriv_no_volume", [("delta", {"symbol": "R_100", "feed": "deriv", "source": "estimated", "tf": "M5", "from": s(D0 + 8 * H), "to": s(D0 + 10 * H)})]),
        ("delta_real_but_none", [("delta", {**est, "source": "real", "tf": "M5", "from": s(D0), "to": s(D0 + H)})]),
        ("delta_bad_tf_after_source", [("delta", {**est, "source": "real", "tf": "M2", "from": s(D0), "to": s(D0 + H)})]),
        ("delta_bad_tf", [("delta", {**xau, "tf": "M2", "from": s(D0), "to": s(D0 + H)})]),
        ("delta_bad_int", [("delta", {**xau, "tf": "M5", "from": "abc", "to": s(D0 + H)})]),
        ("profile_xau_deriv_m5_sessions", [("profile", {**xau, "tf": "M5", "from": s(D0), "to": s(D0 + 23 * H)})]),
        ("profile_btc_binance_h4_day", [("profile", {**btc, "tf": "H4", "from": s(D0 - 6 * 86400), "to": s(D0)})]),
        ("profile_est_d1_week_rowsize", [("profile", {**est, "tf": "D1", "from": s(D0 - 40 * 86400), "to": s(D0), "row_size": est_row, "va": "68", "developing": "0"})]),
        ("profile_rows_zero", [("profile", {**xau, "tf": "M5", "from": s(D0 + 7 * H), "to": s(D0 + 9 * H), "rows": "0"})]),
        ("footprint_xau_m5_dayfile", [("footprint", {**xau, "tf": "M5", "from": s(D0 + 10 * H), "to": s(D0 + 10 * H + 40 * 60)})]),
        ("footprint_xau_m5_day_busy", [("footprint", {**xau, "tf": "M5", "from": s(D0 + 10 * H), "to": s(D0 + 10 * H + 40 * 60)})]),
        ("footprint_btc_m1_rest_gap", [("footprint", {**btc_bb, "tf": "M1", "from": s(D0 + 86400 + 9 * H), "to": s(D0 + 86400 + 9 * H + 25 * 60)})]),
        ("footprint_btc_m1_manual_nofill", [("footprint", {**btc_bb, "tf": "M1", "from": s(D0 + 86400 + 9 * H), "to": s(D0 + 86400 + 9 * H + 25 * 60), "row_size": "2.5", "fill": "0"})]),
        ("footprint_est_m15", [("footprint", {**est, "tf": "M15", "from": s(D0), "to": s(D0 + 6 * H)})]),
        ("footprint_btc_m1_tick_rows", [("footprint", {**btc_bb, "tf": "M1", "from": s(D0 + 86400 + 9 * H), "to": s(D0 + 86400 + 9 * H + 25 * 60), "row_size": "0.1", "fill": "0"})]),
        ("warm_cache_m15_order", [
            ("delta", {**btc, "tf": "M5", "from": s(D0 + 12 * H + 300), "to": s(D0 + 13 * H)}),
            ("delta", {**btc, "tf": "M5", "from": s(D0 + 11 * H), "to": s(D0 + 13 * H)}),
            ("footprint", {**btc, "tf": "M15", "from": s(D0 + 11 * H + 1800), "to": s(D0 + 12 * H + 1800)}),
        ]),
        ("warm_cache_sequence", [
            ("delta", {**btc, "tf": "M5", "from": s(D0 + 12 * H), "to": s(D0 + 14 * H)}),
            ("delta", {**btc, "tf": "M5", "from": s(D0 + 10 * H), "to": s(D0 + 14 * H + 1800)}),
            ("footprint", {**btc, "tf": "M5", "from": s(D0 + 10 * H), "to": s(D0 + 10 * H + 1800)}),
        ]),
    ]


# trade scenarios (store seeds) for footprint calls, by case name
SCENARIOS = {
    "footprint_xau_m5_dayfile": {"sym": "XAUUSDT", "day": "ready", "slice": (D0 + 10 * H - 120, D0 + 10 * H + 46 * 60), "segments": []},
    "footprint_xau_m5_day_busy": {"sym": "XAUUSDT", "day": "busy", "segments": []},
    "footprint_btc_m1_rest_gap": {"sym": "BTCUSDT", "day": None, "segments": [(D0 + 86400 + 9 * H, D0 + 86400 + 9 * H + 12 * 60 - 1), (D0 + 86400 + 9 * H + 18 * 60, D0 + 86400 + 9 * H + 26 * 60 - 1)]},
    "footprint_btc_m1_manual_nofill": "footprint_btc_m1_rest_gap",
    "footprint_btc_m1_tick_rows": "footprint_btc_m1_rest_gap",
    "warm_cache_m15_order": "warm_cache_sequence",
    "warm_cache_sequence": {"sym": "BTCUSDT", "day": "busy", "segments": []},
}


def _scenario(name):
    sc = SCENARIOS[name]
    return SCENARIOS[sc] if isinstance(sc, str) else sc


class Tape:
    """Ordered seam log: record mode calls through and appends; replay mode pops and checks."""

    def __init__(self, entries=None):
        self.replay = entries is not None
        self.entries = list(entries or [])
        self.pos = 0

    def call(self, seam, req, fn):
        if self.replay:
            if self.pos >= len(self.entries):
                raise AssertionError(f"unrecorded call {seam} {req}")
            e = self.entries[self.pos]
            self.pos += 1
            if e["seam"] != seam or e["req"] != req:
                raise AssertionError(f"tape mismatch at {self.pos - 1}: recorded {e['seam']} {e['req']}, now {seam} {req}")
            if "error" in e:
                cls, msg = e["error"]
                raise type(cls, (Exception,), {})(msg)
            return e["resp"]
        try:
            resp = fn()
        except Exception as ex:
            self.entries.append({"seam": seam, "req": req, "error": [ex.__class__.__name__, str(ex)]})
            raise
        self.entries.append({"seam": seam, "req": req, "resp": resp})
        return resp


def _frozen_modules(ot, ofa):
    real_time = __import__("time")

    class FrozenDT(_dtm.datetime):
        @classmethod
        def now(cls, tz=None):
            return _dtm.datetime.fromtimestamp(NOW, tz or _dtm.timezone.utc)

    shim_dt = types.SimpleNamespace(datetime=FrozenDT, timedelta=_dtm.timedelta, timezone=_dtm.timezone, date=_dtm.date)
    shim_time = types.SimpleNamespace(time=lambda: float(NOW), sleep=real_time.sleep)
    ot._dt = shim_dt
    ot.time = shim_time
    ofa.time = shim_time


def _blob(cols):
    """Trade window columns as a content-addressed zip of one CSV (t_ms,price,qty,side; repr floats)."""
    t, p, q, s = cols
    buf = _io.StringIO()
    buf.write("t_ms,price,qty,side\n")
    for i in range(len(t)):
        buf.write(f"{int(t[i])},{float(p[i])!r},{float(q[i])!r},{int(s[i])}\n")
    data = buf.getvalue().encode()
    name = hashlib.sha256(data).hexdigest()[:16]
    path = os.path.join(FIX, "blobs", name + ".zip")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    z = _io.BytesIO()
    with zipfile.ZipFile(z, "w", zipfile.ZIP_DEFLATED) as zf:
        zi = zipfile.ZipInfo("window.csv", date_time=(2026, 1, 1, 0, 0, 0))
        zi.compress_type = zipfile.ZIP_DEFLATED
        zf.writestr(zi, data)
    with open(path, "wb") as f:
        f.write(z.getvalue())
    return "blobs/" + name + ".zip"


def _route(kind, q, hist_for, ofa, rec):
    """hub/server.py:921-968, copied so the history call order is the route's own."""
    now = int(ofa.time.time())
    src = q.get("feed") or "binance"
    hist = hist_for(src)
    if kind == "delta":
        rows, info = ofa.delta(q.get("symbol", "frxXAUUSD"), q.get("tf", "M5"), int(q.get("from", now - 86400)),
                               int(q.get("to", now)), q.get("source", "auto"), history=hist)
        return {"rows": rows, **info}
    if kind == "footprint":
        tf = q.get("tf", "M5"); sym = q.get("symbol", "frxXAUUSD")
        a, b = int(q.get("from", now - 3 * 3600)), int(q.get("to", now))
        sec = ofa.TF_SEC[tf]
        n = min(1000, max(5, (b - a) // sec + 5))
        rec["route_history"] = {"ticker": sym, "tf": tf, "n": n, "to_ms": b * 1000}
        bars = hist(sym, tf, n, b * 1000)
        rec["route_history"]["bars"] = bars
        cb = [(x["time"], x["high"], x["low"]) for x in bars if a <= x["time"] <= b]
        rows, info = ofa.footprint(sym, tf, a, b, q.get("source", "auto"), history=hist,
                                   row_size=float(q.get("row_size", 0)), chart_bars=cb, fill=q.get("fill", "1") != "0")
        return {"candles": rows, **info}
    sess, info = ofa.profile(q.get("symbol", "frxXAUUSD"), q.get("tf", "M5"), int(q.get("from", now - 86400)),
                             int(q.get("to", now)), q.get("source", "auto"), history=hist,
                             mode=q.get("mode", "sessions"), rows=int(q.get("rows", 24)),
                             row_size=float(q.get("row_size", 0)), va_pct=float(q.get("va", 70)) / 100,
                             developing=q.get("developing", "1") == "1")
    return {"sessions": sess, **info}


ROUTE_LINES = [  # the route source this script mirrors; a change in server.py must fail here
    'n = min(1000, max(5, (b - a) // sec + 5))',
    'cb = [(x["time"], x["high"], x["low"]) for x in hist(sym, tf, n, b * 1000) if a <= x["time"] <= b]',
    'hist = {"mt5": mt5_history, "deriv": deriv_history, "arm": arm_history, "bybit": bybit_history}.get(src, binance_history)',
    'row_size=float(q.get("row_size", 0)), chart_bars=cb, fill=q.get("fill", "1") != "0")',
    'mode=q.get("mode", "sessions"), rows=int(q.get("rows", 24)),',
]


def _check_route_source():
    text = open(os.path.join(ROOT, "hub", "server.py"), encoding="utf-8").read()
    for line in ROUTE_LINES:
        if line not in text:
            sys.exit(f"hub/server.py changed; update _route(): missing {line!r}")


def _seed_store(ot, of_mod, sc, inputs, tmp):
    """A TradeStore exactly as Python builds one, from saved inputs, with its loaders disabled."""
    ot.CACHE = os.path.join(tmp, "rest_trades")
    store = ot.TradeStore(sc["sym"])
    store.thread = "frozen"                        # trades() must not start the REST loops
    for seg, pages in zip(sc["segments"], inputs.get("segments", [])):
        rows = [r for page in pages for r in page if seg[0] * 1000 <= r["T"] <= seg[1] * 1000 + 999]
        if rows:
            store._add(ot._arr(rows), seg[0] * 1000, seg[1] * 1000 + 999)
    if sc["day"] == "ready":
        of_mod.CACHE = __import__("pathlib").Path(tmp) / "binance"
        zbytes = open(os.path.join(INPUTS, inputs["daily"]), "rb").read()
        of_mod._get = lambda url, timeout=120, z=zbytes: z
        store._day_job(D)
    elif sc["day"] == "busy":
        store.day_busy.add(D)
    return store


def _store_state(store):
    return {"floor_ms": store.floor_ms, "day_busy": sorted(d.isoformat() for d in store.day_busy),
            "days_ready": sorted(d.isoformat() for d in store.days)}


def run_cases(tapes=None, write=True, only=None):
    """Run every case. Without `tapes` (record): call the network and return the tapes.
    With `tapes` (replay): sockets off, answers from the tapes; write cases/ and stores/."""
    _check_route_source()
    ofa, ot = _hub()
    import orderflow as of_mod
    import server
    manifest = json.load(open(os.path.join(INPUTS, "manifest.json")))
    fut = set(manifest["futures"])
    ofa.futures_symbols = lambda: fut
    _frozen_modules(ot, ofa)
    if tapes is not None:
        def no_net(*a, **k):
            raise RuntimeError("network disabled in replay")
        socket.socket.connect = no_net
    out_tapes = {}
    for name, calls in case_specs(manifest["estimate_symbol"], manifest["estimate_row_size"]):
        if only and name not in only:
            continue
        tape = Tape(None if tapes is None else tapes[name])
        ofa._cache.clear()
        tmp = tempfile.mkdtemp(prefix="ofcap-")
        sc = SCENARIOS.get(name) and _scenario(name)
        src_name = SCENARIOS[name] if isinstance(SCENARIOS.get(name), str) else name
        inp_path = os.path.join(INPUTS, "scenarios", src_name + ".json")
        inputs = json.load(open(inp_path)) if sc and os.path.exists(inp_path) else {}
        store = _seed_store(ot, of_mod, sc, inputs, tmp) if sc else None
        cur = {}                                   # the call being recorded

        def klines_seam(url, w, timeout=20, low=False, _orig=ot._Weight.get, _w=ot.WEIGHT):
            req = {"url": url.replace(ot.FAPI, "FAPI"), "w": w}
            page = tape.call("klines", req, lambda: _orig(_w, url, w, timeout=timeout, low=low))
            cur["fetch"].setdefault("pages", []).append(page)
            return page
        ot.WEIGHT.get = klines_seam
        ofa._ot.WEIGHT.get = klines_seam

        def hist_for(feed):
            fn = {"deriv": server.deriv_history, "bybit": server.bybit_history}.get(feed, server.binance_history)
            def h(ticker, tf, n, to_ms, fn=fn, feed=feed):
                req = {"feed": feed, "ticker": ticker, "tf": tf, "n": n, "to_ms": to_ms}
                bars = tape.call("history", req, lambda: fn(ticker, tf, n, to_ms))
                if cur.get("fetch") is not None and cur["fetch"].get("kind") == "estimated":
                    cur["fetch"].setdefault("chunks", []).append(bars)
                    cur["fetch"].setdefault("calls", []).append([n, to_ms])
                return bars
            return h

        orig_rows, orig_br, orig_er = ofa._rows, ofa._binance_rows, ofa._estimated_rows

        def rows_wrap(source, ticker, itf, a, b, history):
            entry = {"src": source, "ticker": ticker, "tf": itf, "a": a, "b": b, "fetches": []}
            cur["rows_calls"].append(entry)
            cur["rows_entry"] = entry
            return orig_rows(source, ticker, itf, a, b, history)

        def br_wrap(sym, itf, a, b):
            cur["fetch"] = {"kind": "real", "lo": a, "hi": b}
            cur["rows_entry"]["fetches"].append(cur["fetch"])
            try:
                return orig_br(sym, itf, a, b)
            finally:
                cur["fetch"] = None

        def er_wrap(history, ticker, itf, a, b):
            cur["fetch"] = {"kind": "estimated", "lo": a, "hi": b}
            cur["rows_entry"]["fetches"].append(cur["fetch"])
            try:
                return orig_er(history, ticker, itf, a, b)
            finally:
                cur["fetch"] = None
        ofa._rows, ofa._binance_rows, ofa._estimated_rows = rows_wrap, br_wrap, er_wrap

        store_calls = []
        if store is not None:
            orig_trades = store.trades

            def trades_seam(t0, t1, _o=orig_trades):
                (t, p, q, s), cov, pending = _o(t0, t1)
                entry = {"sym": store.sym, "t0": t0, "t1": t1, "window": _blob((t, p, q, s)),
                         "covered": [list(c) for c in cov], "pending": bool(pending), "after": _store_state(store)}
                cur["trades"] = entry
                store_calls.append(entry)
                return (t, p, q, s), cov, pending
            store.trades = trades_seam
            ot.store = lambda sym, s=store: s
            ofa._ot.store = ot.store

        results = []
        for kind, query in calls:
            cur.clear()
            cur.update({"rows_calls": [], "fetch": None})
            rec = {}
            try:
                body = {"status": 200, "body": _route(kind, query, hist_for, ofa, rec)}
            except Exception as e:
                body = {"status": 502, "body": {"error": f"{e.__class__.__name__}: {e}"}}
            results.append({"kind": kind, "query": query, "route_history": rec.get("route_history"),
                            "rows_calls": cur["rows_calls"], "trades": cur.get("trades"), "expected": body})
        ofa._rows, ofa._binance_rows, ofa._estimated_rows = orig_rows, orig_br, orig_er
        shutil.rmtree(tmp, ignore_errors=True)
        if tapes is not None and tape.pos != len(tape.entries):
            raise AssertionError(f"{name}: {len(tape.entries) - tape.pos} recorded calls not made in replay")
        out_tapes[name] = tape.entries
        if write and tapes is not None:
            dump(os.path.join(FIX, "cases", name + ".json"), {"name": name, "now": float(NOW), "calls": results})
            if store is not None:
                dump(os.path.join(FIX, "stores", name + ".json"), {
                    "name": name, "now": float(NOW), "sym": store.sym, "scenario": {k: v for k, v in _scenario(name).items()},
                    "inputs": inputs, "calls": store_calls})
        print(f"{name}: " + ", ".join(f"{r['kind']} {r['expected']['status']}" for r in results))
    return out_tapes


def _id_at(ot, sym, t_ms):
    """First aggTrade id at or after t_ms. Binance only searches startTime over the last 2 days,
    so older starts are found by binary search on fromId (one-trade pages)."""
    latest = ot.WEIGHT.get(f"{ot.FAPI}/aggTrades?symbol={sym}&limit=1", 20, low=True)[-1]["a"]
    lo, hi = 0, latest
    while lo < hi:
        mid = (lo + hi) // 2
        page = ot.WEIGHT.get(f"{ot.FAPI}/aggTrades?symbol={sym}&limit=1&fromId={mid}", 20, low=True)
        if page and page[0]["T"] < t_ms:
            lo = mid + 1
        else:
            hi = mid
    return lo


def _agg_pages(ot, sym, t0_ms, t1_ms):
    """REST aggTrades from t0 through t1, 1000 per page by fromId."""
    pages, from_id = [], _id_at(ot, sym, t0_ms)
    while True:
        q = f"&fromId={from_id}"
        page = ot.WEIGHT.get(f"{ot.FAPI}/aggTrades?symbol={sym}&limit=1000{q}", 20, low=True)
        if not page:
            break
        pages.append(page)
        if page[-1]["T"] > t1_ms or len(page) < 1000:
            break
        from_id = page[-1]["a"] + 1
    return pages


def _rest_rows_from_daily(of_mod, sym, segments):
    """REST-format aggTrades rows ({"a","p","q","T","m"}, strings kept byte for byte) for each
    segment, in pages of 1000. Binance REST only reaches the last 2 days (code -4166), so the rows
    come from the daily archive of the segment's day, which carries the same fields."""
    day = _dtm.datetime.fromtimestamp(segments[0][0], _dtm.timezone.utc).date()
    raw = of_mod._get(f"{of_mod.BASE}/aggTrades/{sym}/{sym}-aggTrades-{day.isoformat()}.zip")
    with zipfile.ZipFile(_io.BytesIO(raw)) as z:
        text = z.open(z.namelist()[0]).read().decode()
    lines = text.splitlines()
    head = lines[0].split(",")
    ix = {k: head.index(k) for k in ("agg_trade_id", "price", "quantity", "transact_time", "is_buyer_maker")}
    out = []
    for a, b in segments:
        rows = []
        for line in lines[1:]:
            c = line.split(",")
            t = int(c[ix["transact_time"]])
            if a * 1000 <= t <= b * 1000 + 999:
                rows.append({"a": int(c[ix["agg_trade_id"]]), "p": c[ix["price"]], "q": c[ix["quantity"]], "T": t,
                             "m": c[ix["is_buyer_maker"]].strip().lower() == "true"})
        out.append([rows[i:i + 1000] for i in range(0, len(rows), 1000)])
    return out


def _slice_daily(of_mod, sym, a_s, b_s):
    """Download the daily zip for D once and keep only rows inside [a_s, b_s] (header kept)."""
    raw = of_mod._get(f"{of_mod.BASE}/aggTrades/{sym}/{sym}-aggTrades-{D.isoformat()}.zip")
    with zipfile.ZipFile(_io.BytesIO(raw)) as z:
        text = z.open(z.namelist()[0]).read().decode()
    lines = text.splitlines()
    head, rows = lines[0], lines[1:]
    col = head.split(",").index("transact_time")
    keep = [r for r in rows if a_s * 1000 <= int(r.split(",")[col]) <= b_s * 1000 + 999]
    data = (head + "\n" + "\n".join(keep) + "\n").encode()
    z = _io.BytesIO()
    with zipfile.ZipFile(z, "w", zipfile.ZIP_DEFLATED) as zf:
        zi = zipfile.ZipInfo(f"{sym}-aggTrades-{D.isoformat()}.csv", date_time=(2026, 1, 1, 0, 0, 0))
        zi.compress_type = zipfile.ZIP_DEFLATED
        zf.writestr(zi, data)
    rel = f"daily/{sym}-aggTrades-{D.isoformat()}-slice.zip"
    path = os.path.join(INPUTS, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(z.getvalue())
    return rel, len(keep)


def cmd_record_only(names):
    """Re-record the tapes of the named cases (inputs and manifest kept)."""
    for name, entries in run_cases(None, write=False, only=set(names)).items():
        dump(os.path.join(INPUTS, "tapes", name + ".json"), entries)


def cmd_record():
    """Pass 1 (network): futures list, estimate-only symbol, trade scenario inputs, then every case's tape."""
    ofa, ot = _hub()
    import orderflow as of_mod
    import server
    os.makedirs(INPUTS, exist_ok=True)
    fut = sorted(ofa.futures_symbols())
    if not fut:
        sys.exit("exchangeInfo failed")
    est_sym, est_row = None, None
    for cand in EST_CANDIDATES:
        base = cand[len("BYBIT_"):-4]
        if base + "USDT" in fut:
            continue
        try:
            bars = server.bybit_history(cand, "D1", 5, None)
        except Exception:
            continue
        if bars and any(b["volume"] for b in bars):
            est_sym = cand
            est_row = repr(float(ofa.nice_step(bars[-1]["close"] / 40)))
            break
    if not est_sym:
        sys.exit("no estimate-only Bybit symbol found among the candidates")
    dump(os.path.join(INPUTS, "manifest.json"), {
        "python": sys.version.split()[0], "date": D.isoformat(), "now": NOW, "futures": fut,
        "estimate_symbol": est_sym, "estimate_row_size": est_row})
    print("estimate-only symbol", est_sym, "row_size", est_row)
    done = set()
    for name in SCENARIOS:
        sc = _scenario(name)
        key = json.dumps(sc, sort_keys=True)
        target = os.path.join(INPUTS, "scenarios", name + ".json")
        inputs = {}
        if sc["day"] == "ready":
            rel, n = _slice_daily(of_mod, sc["sym"], *sc["slice"])
            inputs["daily"] = rel
            print(f"{name}: daily slice {n} trades")
        if sc["segments"]:
            inputs["segments"] = _rest_rows_from_daily(of_mod, sc["sym"], sc["segments"])
            print(f"{name}: {sum(len(p) for s in inputs['segments'] for p in s)} REST trades")
        dump(target, inputs)
        done.add(key)
    tapes = run_cases(None, write=False)
    for name, entries in tapes.items():
        dump(os.path.join(INPUTS, "tapes", name + ".json"), entries)
    print("pass 1 done")


def cmd_replay():
    tapes = {}
    for f in sorted(os.listdir(os.path.join(INPUTS, "tapes"))):
        tapes[f[:-5]] = json.load(open(os.path.join(INPUTS, "tapes", f)))
    for sub in ("cases", "stores", "blobs"):
        shutil.rmtree(os.path.join(FIX, sub), ignore_errors=True)
    run_cases(tapes, write=True)
    print("pass 2 done")


def _tree_digest(root):
    out = {}
    for d, _, files in os.walk(root):
        for f in files:
            p = os.path.join(d, f)
            rel = os.path.relpath(p, root).replace(os.sep, "/")
            if rel.startswith("inputs/"):
                continue
            if f.endswith(".zip"):
                with zipfile.ZipFile(p) as z:
                    data = b"".join(z.read(n) for n in sorted(z.namelist()))
            else:
                data = open(p, "rb").read()
            out[rel] = hashlib.sha256(data).hexdigest()
    return out


def cmd_check():
    before = _tree_digest(FIX)
    cmd_numeric()
    cmd_replay()
    after = _tree_digest(FIX)
    diff = sorted(set(before.items()) ^ set(after.items()))
    size = sum(os.path.getsize(os.path.join(d, f)) for d, _, fs in os.walk(FIX) for f in fs)
    print(f"fixture tree {size / 1e6:.2f} MB")
    if diff:
        sys.exit(f"fixtures changed on re-run: {sorted({p for p, _ in diff})}")
    if size > 8_000_000:
        sys.exit("fixture tree over the 8 MB budget")
    print("check: identical")


def main(argv):
    need_312()
    cmd = argv[1] if len(argv) > 1 else ""
    if cmd == "numeric":
        cmd_numeric()
    elif cmd == "record" and len(argv) > 2:
        cmd_record_only(argv[2:])
    elif cmd == "record":
        cmd_record()
    elif cmd == "replay":
        cmd_replay()
    elif cmd == "check":
        cmd_check()
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
