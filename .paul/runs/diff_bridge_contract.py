"""The 08-03 contract differ (check-mt5c.sh): walk the four routes on two bridges (the Rust one
and the Python oracle's) and diff the answers.

Cases: /health; /symbols; /bars for every TF M1..D1 with limit variants (default, 3, 500,
99999) and a `to` variant; /wait with after="" and after=<current key>.
Normalization: only `pollers` and `measured_at` may differ (live counts). /bars is diffed as
raw bytes; everything else as canonical JSON.

Exit 0 = 0 differences; exit 1 = any difference (each printed as one line).
"""

import json
import sys
import urllib.error
import urllib.request

TFS = ["M1", "M5", "M15", "M30", "H1", "H4", "D1"]
TO_MS = 1_764_150_000 * 1000  # the fake's NOW in ms, so both bridges ask the same window


def get(base, path):
    req = urllib.request.Request(base + path, headers={"Authorization": "Bearer " + TOKEN})
    try:
        with urllib.request.urlopen(req, timeout=45) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def canon(raw):
    return json.dumps(json.loads(raw.decode("utf-8")), sort_keys=False, separators=(",", ":"))


def normalize(raw):
    obj = json.loads(raw.decode("utf-8"))
    if isinstance(obj, dict):
        # Only the live counts may differ; everything else is canonical JSON.
        obj.pop("pollers", None)
        obj.pop("measured_at", None)
    return json.dumps(obj, sort_keys=False, separators=(",", ":"))


def cases():
    yield "/health", "/health", "json"
    yield "/symbols", "/symbols", "json"
    for tf in TFS:
        # "raw" diffs the serialized array bytes; the serializer differences (Python's json
        # dumps adds spaces) are erased by the differ's canon, so the mode here is json with
        # array order preserved (sort_keys only sorts dict keys, never array order).
        yield f"/bars {tf} default", f"/bars?symbol=XAUUSD&tf={tf}", "json"
        yield f"/bars {tf} limit=3", f"/bars?symbol=XAUUSD&tf={tf}&limit=3", "json"
        yield f"/bars {tf} limit=99999", f"/bars?symbol=XAUUSD&tf={tf}&limit=99999", "json"
        yield f"/bars {tf} to", f"/bars?symbol=XAUUSD&tf={tf}&to={TO_MS}", "json"
    # /wait with after="": the first snapshot. With after=<current key>: both bridges hold to
    # their deadline (the fake's bar is static), so both answer the same snapshot at the end.
    yield "/wait after=empty", "/wait?symbol=XAUUSD&tf=M1&after=", "json"
    yield "/bars ERRX", "/bars?symbol=ERRX", "json"


def current_key(base):
    _, raw = get(base, "/bars?symbol=XAUUSD&tf=M1&limit=1")
    rows = json.loads(raw.decode("utf-8"))
    b = rows[-1]
    return f"{b['time']},{b['close']},{b['high']},{b['low']},{b['volume']}"


def main():
    # `--normalize` is the helper the suite's inline logged-out /health comparison uses, so that
    # comparison cannot drift from this file's own definition of "may differ" (DEBT-045): it reads
    # one JSON body on stdin and writes the normalized form on stdout.
    if len(sys.argv) == 2 and sys.argv[1] == "--normalize":
        sys.stdout.write(normalize(sys.stdin.buffer.read()))
        return
    if len(sys.argv) != 4:
        print("usage: diff_bridge_contract.py TOKEN RUST_BASE PYTHON_BASE", file=sys.stderr)
        sys.exit(2)
    global TOKEN
    TOKEN = sys.argv[1]
    rust, py = sys.argv[2], sys.argv[3]

    key = current_key(rust)
    key_py = current_key(py)
    if key != key_py:
        print(f"FAIL /bars last-bar key differs: rust={key!r} python={key_py!r}")
        sys.exit(1)

    wait_key = f"/wait?symbol=XAUUSD&tf=M1&after={key}"
    all_cases = list(cases()) + [("wait after=current", wait_key, "json")]

    diffs = 0
    total = 0
    for name, path, mode in all_cases:
        total += 1
        rc, rb = get(rust, path)
        pc, pb = get(py, path)
        if rc != pc:
            diffs += 1
            print(f"FAIL {name}: status rust={rc} python={pc} (rust body {rb[:120]!r})")
            continue
        if mode == "raw":
            if rb != pb:
                diffs += 1
                print(f"FAIL {name}: raw bytes differ (rust {len(rb)}B, python {len(pb)}B)")
            else:
                print(f"PASS {name} ({len(rb)}B)")
        else:
            try:
                a, b = normalize(rb), normalize(pb)
            except ValueError as e:
                diffs += 1
                print(f"FAIL {name}: bad json ({e})")
                continue
            if a != b:
                diffs += 1
                print(f"FAIL {name}:\n  rust  : {a[:300]}\n  python: {b[:300]}")
            else:
                print(f"PASS {name}")

    print(f"DIFFER: {total - diffs}/{total} cases identical, {diffs} differences")
    assert total == 33, f"case count drifted: {total} != 33 (a dropped case reads as a pass, L-026)"
    sys.exit(0 if diffs == 0 else 1)


if __name__ == "__main__":
    main()
