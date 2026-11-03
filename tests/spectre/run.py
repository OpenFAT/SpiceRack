#!/usr/bin/env python3
"""T-S1 to T-S3 of docs/research/monte-carlo-spectre.md §10.6 on a real Spectre.

Each deck here is what SpiceRack writes for a SPICE source (regenerated and
checked by tests/test_monte_carlo.rs). This script runs one deck, reads its
operating points (the nominal `op1` and, under `montecarlo`, every `dcmc`
iteration), and checks them against expected.json: the closed forms and the
ngspice reference, at the tolerances recorded there.

    python3 run.py t_s2_agauss.scs          # run spectre, then check
    python3 run.py t_s2_agauss.scs --raw DIR  # check an existing raw directory

Prints one PASS/FAIL line per check and a JSON summary to send back. Needs
`spectre` on PATH (it is called with `-format psfascii`).
"""

import json
import math
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PAIR = re.compile(r'^\s*"([^"]+)"\s+([-+0-9.eE]+)\s*$')


def values(path):
    """Every `"name" number` row of a psfascii file, in order."""
    out = {}
    with open(path, errors="replace") as f:
        for line in f:
            m = PAIR.match(line)
            if m:
                try:
                    out.setdefault(m.group(1), []).append(float(m.group(2)))
                except ValueError:
                    pass
    return out


def collect(raw):
    """(nominal values, per-iteration values) from a Spectre raw directory."""
    nominal, runs = {}, {}
    for root, _, files in os.walk(raw):
        for name in sorted(files):
            path = os.path.join(root, name)
            v = values(path)
            if "dcmc" in name:
                for k, xs in v.items():
                    runs.setdefault(k, []).extend(xs)
            elif name.startswith("op1"):
                nominal.update({k: xs[0] for k, xs in v.items()})
    return nominal, runs


def stats(xs):
    n = len(xs)
    m = sum(xs) / n
    s = math.sqrt(sum((x - m) ** 2 for x in xs) / (n - 1)) if n > 1 else 0.0
    return m, s


def rho(a, b):
    (ma, sa), (mb, sb) = stats(a), stats(b)
    n = min(len(a), len(b))
    return sum((a[i] - ma) * (b[i] - mb) for i in range(n)) / (n - 1) / (sa * sb)


def main():
    deck = sys.argv[1]
    want = json.load(open(os.path.join(HERE, "expected.json")))[os.path.basename(deck)]
    raw = sys.argv[sys.argv.index("--raw") + 1] if "--raw" in sys.argv else os.path.splitext(deck)[0] + ".raw"
    if "--raw" not in sys.argv:
        subprocess.run(["spectre", "-format", "psfascii", "-raw", raw, os.path.join(HERE, deck)], check=True)
    nominal, runs = collect(raw)
    summary = {"deck": deck, "nominal": nominal, "runs": {k: len(v) for k, v in runs.items()}}
    ok = True

    def check(name, good, detail):
        nonlocal ok
        ok &= good
        print(f"{'PASS' if good else 'FAIL'} {deck} {name}: {detail}")

    if "nominal" in want:
        for k, v in want["nominal"].items():
            got = nominal.get(k)
            check(f"nominal {k}", got is not None and abs(got - v) <= want["rel_tol"] * abs(v), f"{got} vs {v}")
    if "probe" in want:
        xs = runs.get(want["probe"], [])
        if not xs:
            check("iterations", False, f"no {want['probe']} values; files: {sorted(os.listdir(raw))}")
        else:
            m, s = stats(xs)
            summary["stats"] = {"n": len(xs), "mean": m, "std": s}
            check("mean", abs(m - want["mean"]) < want["mean_tol"], f"{m:.6e} vs {want['mean']:.6e} (ngspice {want['ngspice']['mean']:.6e})")
            check("std", abs(s - want["std"]) < want["std_tol"], f"{s:.6e} vs {want['std']:.6e} (ngspice {want['ngspice']['std']:.6e})")
            if "values" in want:
                off = [x for x in xs if min(abs(x - v) for v in want["values"]) > want["value_tol"]]
                check("two values", not off, f"{len(off)} values off {want['values']}")
    if "pairs" in want:
        summary["rho"] = {}
        for a, b in want["pairs"]:
            xa, xb = runs.get(a, []), runs.get(b, [])
            if not xa or not xb:
                check(f"rho {a},{b}", False, f"no values; files: {sorted(os.listdir(raw))}")
                continue
            r = rho(xa, xb)
            summary["rho"][f"{a},{b}"] = r
            check(f"rho {a},{b}", abs(r) < want["abs_rho_below"], f"{r:+.4f}")
        for k, s0 in want["std"].items():
            xs = runs.get(k, [])
            if xs:
                s = stats(xs)[1]
                check(f"std {k}", abs(s - s0) < want["std_tol"], f"{s:.6e} vs {s0:.6e}")
    print(json.dumps(summary, indent=1))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
