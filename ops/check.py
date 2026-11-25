#!/usr/bin/env python3
"""ops/check.py: the ops surface's structural gate.

The plan's AC-1 and AC-2 are claims about files the CI and this machine cannot execute (systemd
units, cloudflared config, a drill that needs a live Postgres). This script is what makes them
checkable: it parses the files and asserts the properties, and it is the mutation target for m2.

    python ops/check.py            # exit 0 pass, 1 on the first failing assertion
    python ops/check.py --quiet    # only the verdict

Checks, in order:
  1. every file the runbook names exists
  2. vtv.service carries the hardening flags AC-1 lists, runs as vtv, and reads /etc/vtv/env
  3. cloudflared.yml has exactly one hostname rule to the loopback origin and a 404 catch-all
  4. backup.sh and restore_drill.sh pass their own --check
  5. no file under ops/ contains a literal secret (a long base64/hex blob, a password=, a token=)
"""

import re
import shutil
import subprocess
import sys
from pathlib import Path

OPS = Path(__file__).resolve().parent
ROOT = OPS.parent
FAILS = []


def bash_binary():
    """A real bash, not Windows' WSL stub (System32\\bash.exe fails without a distro)."""
    for cand in (
        r"C:\Program Files\Git\bin\bash.exe",
        r"C:\Program Files\Git\usr\bin\bash.exe",
        "/bin/bash",
    ):
        if Path(cand).is_file():
            return cand
    found = shutil.which("bash")
    return found or "bash"


def check(cond, message):
    if not cond:
        FAILS.append(message)
    return cond


def read(name):
    p = OPS / name
    if not p.is_file():
        FAILS.append(f"missing file: ops/{name}")
        return ""
    return p.read_text(encoding="utf-8", errors="replace")


def body_without_comments(text):
    return "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("#"))


def units(text):
    """(section, key) -> value for a systemd-style ini file."""
    sec, out = None, {}
    for line in text.splitlines():
        s = line.strip()
        if not s or s.startswith(("#", ";")):
            continue
        if s.startswith("[") and s.endswith("]"):
            sec = s[1:-1]
            continue
        if "=" in s:
            k, v = s.split("=", 1)
            out[(sec, k.strip())] = v.strip()
    return out


def main():
    quiet = "--quiet" in sys.argv

    # 1. the runbook's file list
    for name in [
        "README.md", "cloudflared.yml", "vtv.service", "vtv-backup.service",
        "vtv-backup.timer", "backup.sh", "restore_drill.sh", "deploy.sh", "check.py",
    ]:
        check((OPS / name).is_file(), f"ops/{name} missing")

    # 2. the service unit
    svc = units(read("vtv.service"))
    want = {
        ("Service", "User"): "vtv",
        ("Service", "EnvironmentFile"): "/etc/vtv/env",
        ("Service", "NoNewPrivileges"): "true",
        ("Service", "ProtectSystem"): "strict",
        ("Service", "PrivateTmp"): "true",
        ("Service", "Restart"): "always",
        ("Service", "ProtectHome"): "read-only",
        ("Service", "CapabilityBoundingSet"): "",
        ("Service", "RestrictAddressFamilies"): "AF_INET AF_INET6 AF_UNIX",
        ("Service", "ReadWritePaths"): "/opt/vtv/hub/logs /opt/vtv/hub/.cache/orderflow",
    }
    for key, val in want.items():
        check(svc.get(key) == val, f"vtv.service [{key[0]}] {key[1]} = {val!r} (got {svc.get(key)!r})")

    # 3. the tunnel config
    cf = body_without_comments(read("cloudflared.yml"))
    check("http_status:404" in cf, "cloudflared.yml: no 404 catch-all")
    check("127.0.0.1:8770" in cf, "cloudflared.yml: origin is not our loopback port")
    check(cf.count("- hostname:") == 1, "cloudflared.yml: expected exactly one hostname rule")
    check(cf.count("service:") == 2, "cloudflared.yml: expected two service lines")
    check("tunnel: vtradingview" in cf, "cloudflared.yml: tunnel name")

    # 4. the scripts' own structure checks
    bash = bash_binary()
    for name in ("backup.sh", "restore_drill.sh"):
        p = OPS / name
        if p.is_file():
            r = subprocess.run([bash, str(p), "--check"], capture_output=True, text=True)
            check(r.returncode == 0, f"{name} --check failed: {(r.stdout + r.stderr).strip()[:200]}")

    # 5. no literal secrets
    secretish = re.compile(
        r"(?:password|secret|token)\s*=\s*[A-Za-z0-9+/=_-]{16,}|[A-Za-z0-9+/]{40,}={0,2}"
    )
    allowed_context = ("<", "placeholder", "random bytes", "generator", "example", "REPO_URL",
                       "tunnel token", "Access Key")
    for f in sorted(OPS.glob("*")):
        if not f.is_file() or f.suffix not in (".sh", ".md", ".yml", ".service", ".timer"):
            continue
        for i, line in enumerate(f.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            if not secretish.search(line):
                continue
            if any(a.lower() in line.lower() for a in allowed_context):
                continue
            if line.lstrip().startswith(("#", "//")):
                continue
            FAILS.append(f"{f.name}:{i} looks like a literal secret")

    if FAILS:
        for msg in FAILS:
            print("FAIL:", msg)
        print(f"ops check: FAIL ({len(FAILS)} assertion(s))")
        return 1
    if not quiet:
        print("ops check: OK (files, unit hardening, tunnel ingress, script self-tests, no secrets)")
    else:
        print("ops check: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
