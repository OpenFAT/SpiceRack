"""spicerack.remeasure against closed forms, never against the code's own method.

Oracles: tones of known amplitude (SNDR/SFDR/THD from their powers, no FFT),
a first-order step (tau*ln(1/tol)), linear ramps (exact slew), two- and
three-pole loops (analytic margins), power-law densities (exact integrals),
kT/C from a real ngspice noise run, and Clopper-Pearson end points in closed
form plus a brute-force binomial reference. Raw files are written in nutmeg
binary and read back through the real reader.

Run: maturin develop && pytest tests/test_remeasure.py -v
"""
import math
import shutil
import struct
import subprocess

import numpy as np
import pytest

from spicerack import remeasure as rm


def write_raw(path, plots):
    """``plots``: (plotname, names, columns); complex columns make a complex plot."""
    out = b""
    for plotname, names, cols in plots:
        cplx = any(np.iscomplexobj(c) for c in cols)
        npts = len(cols[0])
        head = (f"Title: remeasure test\nPlotname: {plotname}\nFlags: {'complex' if cplx else 'real'}\n"
                f"No. Variables: {len(names)}\nNo. Points: {npts}\nVariables:\n")
        head += "".join(f"\t{i}\t{n}\tvoltage\n" for i, n in enumerate(names)) + "Binary:\n"
        data = []
        for p in range(npts):
            for c in cols:
                v = complex(c[p])
                data += [v.real, v.imag] if cplx else [v.real]
        out += head.encode() + struct.pack(f"<{len(data)}d", *data)
    path.write_bytes(out)
    return rm.read(path)


# ── sndr ──

N, M, FS = 4096, 251, 1.0
A, HARM, NOISE = 1.0, {2: 1e-3, 3: 5e-4}, {100: 2e-4, 1500: 3e-4, 1999: 1e-4}


def tones(cycles, n=N):
    """``A`` at ``cycles``, its harmonics, off-harmonic 'noise' tones and a DC offset, at k = 0..n."""
    k = np.arange(n + 1)
    x = 0.3 + A * np.sin(2 * np.pi * cycles * k / n + 0.4)
    for h, a in HARM.items():
        x += a * np.sin(2 * np.pi * h * cycles * k / n + h)
    for b, a in NOISE.items():
        x += a * np.sin(2 * np.pi * b * k / n + 0.1 * b)
    return k / FS, x


def exact():
    p_sig, p_harm = A ** 2 / 2, sum(a ** 2 for a in HARM.values()) / 2
    p_noise = sum(a ** 2 for a in NOISE.values()) / 2
    spur = max(list(HARM.values()) + list(NOISE.values()))
    return {"sndr_db": 10 * math.log10(p_sig / (p_harm + p_noise)), "sfdr_db": 20 * math.log10(A / spur),
            "thd_db": 10 * math.log10(p_harm / p_sig)}


def test_sndr_coherent_matches_tone_powers(tmp_path):
    raw = write_raw(tmp_path / "s.raw", [("Transient Analysis", ["time", "v(out)"], tones(M))])
    for t0 in (None, 0.0):
        got = rm.sndr(raw, "v(out)", fs=FS, fin=FS * M / N, n=N, t0=t0)
        for key, want in exact().items():
            assert got[key] == pytest.approx(want, abs=1e-9), key
        assert got["enob"] == pytest.approx((got["sndr_db"] - 1.7609125905568) / 6.0205999132796, abs=1e-12)
        assert got["coherent"] and got["signal_bin"] == M and got["signal_bins"] == [M]
        assert got["harmonic_bins"] == {h: [h * M] for h in range(2, 7)}  # 6*251 < 2048: none fold


def test_sndr_refuses_incoherent_unless_windowed(tmp_path):
    raw = write_raw(tmp_path / "s.raw", [("Transient Analysis", ["time", "v(out)"], tones(M + 0.37))])
    with pytest.raises(ValueError, match="incoherent"):
        rm.sndr(raw, "v(out)", fs=FS, fin=FS * (M + 0.37) / N, n=N)
    got = rm.sndr(raw, "v(out)", fs=FS, fin=FS * (M + 0.37) / N, n=N, window="blackmanharris")
    assert not got["coherent"]
    for key, want in exact().items():
        assert got[key] == pytest.approx(want, abs=0.3), key  # windowed: leakage and bin-split peaks

    even = write_raw(tmp_path / "e.raw", [("Transient Analysis", ["time", "v(out)"], tones(256))])
    with pytest.raises(ValueError, match="prime"):
        rm.sndr(even, "v(out)", fs=FS, fin=FS * 256 / N, n=N)  # integer cycles but gcd(256, 4096) != 1


def test_sndr_refuses_a_short_record(tmp_path):
    raw = write_raw(tmp_path / "s.raw", [("Transient Analysis", ["time", "v(out)"], tones(M, n=N // 2))])
    with pytest.raises(ValueError, match="does not hold"):
        rm.sndr(raw, "v(out)", fs=FS, fin=FS * M / N, n=N)


# ── time domain ──

def test_settling_first_order_step(tmp_path):
    tau, t0, final, tol = 1e-6, 1e-6, 0.9, 1e-3
    t = np.linspace(0, 20e-6, 20001)
    v = np.where(t < t0, 0.0, final * (1 - np.exp(-(t - t0) / tau)))
    raw = write_raw(tmp_path / "s.raw", [("Transient Analysis", ["time", "v(out)"], (t, v))])
    assert rm.settling(raw, "v(out)", final=final, tol=tol, t0=t0) == pytest.approx(tau * math.log(1 / tol), rel=1e-5)
    # absolute band: the same step settles into +-0.9 mV at tau*ln(final/0.9e-3)
    assert rm.settling(raw, "v(out)", final=final, t0=t0, abs_tol=0.9e-3) == pytest.approx(
        tau * math.log(final / 0.9e-3), rel=1e-5)
    with pytest.raises(ValueError, match="exactly one band"):
        rm.settling(raw, "v(out)", final=final, tol=tol, t0=t0, abs_tol=1e-3)
    short = write_raw(tmp_path / "short.raw", [("Transient Analysis", ["time", "v(out)"], (t[:5000], v[:5000]))])
    assert rm.settling(short, "v(out)", final=final, tol=tol, t0=t0) == math.inf


def test_slew_of_linear_edges(tmp_path):
    t = np.array([0, 1, 2, 5, 7, 9]) * 1e-6
    v = np.array([0, 0, 1, 1, 0, 0.0])  # 1 V in 1 us up, 1 V in 2 us down
    raw = write_raw(tmp_path / "s.raw", [("Transient Analysis", ["time", "v(out)"], (t, v))])
    got = rm.slew(raw, "v(out)", lo=0.1, hi=0.9)
    assert got["rising"] == pytest.approx(1e6, rel=1e-12)
    assert got["falling"] == pytest.approx(0.5e6, rel=1e-12)
    rise = write_raw(tmp_path / "r.raw", [("Transient Analysis", ["time", "v(out)"], (t[:4], v[:4]))])
    assert math.isnan(rm.slew(rise, "v(out)")["falling"])


# ── loop margins ──

def lstb(path, w_of_f):
    f = np.logspace(0, 9, 9 * 200 + 1)
    w = w_of_f(f)
    names = ["frequency", "loop_gain", "loop_gain_forward"]
    return write_raw(path, [("Loop Stability Analysis", names, (f.astype(complex), w, w))])


def test_loop_margins_two_pole(tmp_path):
    a0, p1, p2 = 1e3, 1e3, 1e6
    raw = lstb(tmp_path / "l.raw", lambda f: a0 / ((1 + 1j * f / p1) * (1 + 1j * f / p2)))
    # |W| = 1: u^2/(p1 p2)^2 + u (1/p1^2 + 1/p2^2) + 1 - a0^2 = 0, u = f^2
    qa, qb, qc = 1 / (p1 * p2) ** 2, 1 / p1 ** 2 + 1 / p2 ** 2, 1 - a0 ** 2
    ugf = math.sqrt((-qb + math.sqrt(qb * qb - 4 * qa * qc)) / (2 * qa))
    pm = 180 - math.degrees(math.atan(ugf / p1) + math.atan(ugf / p2))
    got = rm.loop_margins(raw, plot="Loop Stability Analysis")
    assert got["ugf_hz"] == pytest.approx(ugf, rel=1e-4)
    assert got["pm_deg"] == pytest.approx(pm, abs=1e-3)
    assert math.isnan(got["gm_db"]) and math.isnan(got["pcf_hz"])  # two poles never reach -180
    assert got["unity_crossings"] == 1


def test_loop_margins_three_pole_gain_margin(tmp_path):
    a0, p = 4.0, 1e4
    raw = lstb(tmp_path / "l.raw", lambda f: a0 / (1 + 1j * f / p) ** 3)
    got = rm.loop_margins(raw)
    assert got["pcf_hz"] == pytest.approx(p * math.sqrt(3), rel=1e-4)  # 3 atan(f/p) = 180 deg
    # |W| = a0 / 2^3 there. Linear in ln f over h = ln(10)/200: h^2/8 * |d2 ln|W| / d(ln f)^2| (1.125 at
    # the crossing) = 2e-5 Np on |W| plus as much again through the located crossing, ~3e-4 dB.
    assert got["gm_db"] == pytest.approx(20 * math.log10(8 / a0), abs=1e-3)

    # unstable: (1 + x^2)^(3/2) = 20 at the UGF, PM = 180 - 3 atan(x) < 0, GM < 0
    raw = lstb(tmp_path / "u.raw", lambda f: 20 / (1 + 1j * f / p) ** 3)
    x = math.sqrt(20 ** (2 / 3) - 1)
    got = rm.loop_margins(raw)
    assert got["pm_deg"] == pytest.approx(180 - 3 * math.degrees(math.atan(x)), abs=1e-3)
    assert got["gm_db"] == pytest.approx(20 * math.log10(8 / 20), abs=1e-3)


# ── noise ──

def test_noise_total_is_exact_on_power_laws(tmp_path):
    f = np.logspace(0, 6, 61)  # 10 points/decade: trapezoids would be off by percent
    for s, integral in ((1e-12 / f, 1e-12 * math.log(1e6)),
                        (np.full_like(f, 4e-18), 4e-18 * (1e6 - 1)),
                        (1e-30 * f ** 2, 1e-30 * (1e18 - 1) / 3)):
        raw = write_raw(tmp_path / "n.raw", [("Noise Spectral Density Curves", ["frequency", "onoise_spectrum"],
                                              (f, np.sqrt(s)))])
        assert rm.noise_total(raw) == pytest.approx(math.sqrt(integral), rel=1e-12)


def test_noise_total_rc_against_ktc_on_ngspice(tmp_path):
    """R thermal noise into C: v^2 = kT/C * (2/pi) [atan(f2/fc) - atan(f1/fc)]."""
    assert shutil.which("ngspice"), "ngspice is part of nix develop .#testing"
    r, c, f1, f2 = 1e3, 1e-9, 1.0, 1e9
    deck = tmp_path / "rc.sp"
    deck.write_text(f"rc noise\nV1 in 0 DC 0 AC 1\nR1 in out {r}\nC1 out 0 {c}\n"
                    f".noise v(out) V1 dec 50 {f1} {f2}\n.end\n")
    raw_path = tmp_path / "rc.raw"
    subprocess.run(["ngspice", "-b", "-r", str(raw_path), str(deck)], check=True, capture_output=True, timeout=60)
    raw = rm.read(raw_path)
    fc = 1 / (2 * math.pi * r * c)
    kt = 1.380649e-23 * 300.15
    want = math.sqrt(kt / c * 2 / math.pi * (math.atan(f2 / fc) - math.atan(f1 / fc)))
    assert rm.noise_total(raw) == pytest.approx(want, rel=1e-3)
    # every plot kept: ngspice's own integral sits in the later plot
    totals = [p for p in raw if p.plot_name == "Integrated Noise"]
    assert totals and totals[0]["v(onoise_total)"][0] == pytest.approx(want, rel=1e-3)


# ── yield ──

def binomial_ci(k, n, conf):
    """Reference: bisection on the exact binomial tails (math.comb), independent of the beta function."""
    def tail_ge(p):
        return sum(math.comb(n, i) * p ** i * (1 - p) ** (n - i) for i in range(k, n + 1))

    def bisect(f, target):
        lo, hi = 0.0, 1.0
        for _ in range(100):
            mid = (lo + hi) / 2
            lo, hi = (mid, hi) if f(mid) < target else (lo, mid)
        return lo
    a = (1 - conf) / 2
    low = 0.0 if k == 0 else bisect(tail_ge, a)
    # P(X <= k | p) = a, i.e. P(X >= k+1 | p) = 1 - a, increasing in p
    high = 1.0 if k == n else bisect(lambda p: tail_ge(p) - math.comb(n, k) * p ** k * (1 - p) ** (n - k), 1 - a)
    return low, high


def test_yield_ci_closed_forms_and_reference():
    low, high = rm.yield_ci(passes=2300, n=2300, conf=0.95)
    assert high == 1.0 and low == pytest.approx(0.025 ** (1 / 2300), rel=1e-12)
    low, high = rm.yield_ci(passes=0, n=50, conf=0.9)
    assert low == 0.0 and high == pytest.approx(1 - 0.05 ** (1 / 50), rel=1e-12)
    for k, n, conf in ((7, 20, 0.95), (1, 10, 0.99), (37, 40, 0.9)):
        assert rm.yield_ci(k, n, conf) == pytest.approx(binomial_ci(k, n, conf), abs=1e-9)
    with pytest.raises(ValueError):
        rm.yield_ci(passes=5, n=4)


# ── compare ──

MEASURES = {
    "format": "espice-measures/1", "deck": "ota.sp",
    "results": [
        {"name": "gain", "analysis": "ac", "variant": {"kind": "nominal"}, "value": 144.242},
        {"name": "ugf", "analysis": "ac", "variant": {"kind": "nominal"}, "value": 30304900.0},
        {"name": "ugf", "analysis": "ac", "variant": {"kind": "step", "params": {"ib": 9e-06}}, "value": 1.0},
        {"name": "off", "analysis": "dc", "variant": {"kind": "nominal"}, "value": None,
         "error": "WHEN condition never met"},
    ],
}


def test_compare_lists_every_disagreement(tmp_path):
    assert rm.compare(MEASURES, {"GAIN": 144.25, "ugf": 30.3e6}, rtol=1e-3) == []
    rows = rm.compare(MEASURES, {"gain": 150.0, "off": 0.1, "pm": 60.0, "ugf": 30.3049e6}, rtol=1e-3)
    assert [(r["name"], r["simulator"]) for r in rows] == [("gain", 144.242), ("off", None), ("pm", None)]
    assert "WHEN condition never met" in rows[1]["reason"]
    path = tmp_path / "meas.json"
    path.write_text(__import__("json").dumps(MEASURES))
    assert rm.compare(path, {"gain": float("nan")})[0]["name"] == "gain"
    with pytest.raises(ValueError, match="espice-measures/1"):
        rm.compare({"format": "other", "results": []}, {})


def test_complex_of_a_real_column(tmp_path):
    raw = write_raw(tmp_path / "r.raw", [("AC", ["frequency", "v(a)"], (np.array([1.0, 2.0]), np.array([3.0, 4.0])))])
    assert raw[0].complex("V(A)") == [3 + 0j, 4 + 0j]
