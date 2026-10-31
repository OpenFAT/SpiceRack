"""Deciding numbers recomputed from raw waveforms, independent of the simulator.

A reported pass is a claim until separate code recomputes the deciding number
from the raw data. Every function here reads only the vectors of a ``.raw``
file, never the simulator's ``.meas`` output, and needs no simulator.

```python
from spicerack import remeasure as rm

raw = rm.read("out.raw")      # every plot of the file, in file order
rm.sndr(raw, "v(out)", fs=5.12e6, fin=5.12e6 * 251 / 4096, n=4096)
rm.settling(raw, "v(out)", final=0.9, tol=1e-3, t0=1e-6)
rm.loop_margins(raw, plot="Loop Stability Analysis")
rm.noise_total(raw)
rm.yield_ci(passes=2300, n=2300, conf=0.95)
```

``raw`` is the list ``read`` returns, or one plot. A function looks up its
vector in the first plot that holds it (or in the plot named ``plot=``).
"""

from __future__ import annotations

import json
import math
from pathlib import Path
from typing import Any, Mapping

import numpy as np

from ._native import read_raw

#: 4-term Blackman-Harris: sidelobes at -92 dB, main lobe +-4 bins.
_BLACKMAN_HARRIS = (0.35875, 0.48829, 0.14128, 0.01168)
#: Bins either side of a tone counted as that tone under a window: the
#: +-4-bin main lobe plus one for a tone that falls between bins.
_WINDOW_HALF_WIDTH = 5


def read(path: str | Path) -> list[Any]:
    """Every plot of an ngspice / ESPice / VACASK / LTspice raw file, binary or ASCII."""
    return read_raw(str(path))


def _plot(raw: Any, vec: str, plot: str | None = None) -> Any:
    plots = [raw] if hasattr(raw, "plot_name") else list(raw)
    for p in plots:
        if plot is not None and p.plot_name != plot:
            continue
        if any(v.lower() == vec.lower() for v in p.variable_names):
            return p
    where = f" '{plot}'" if plot is not None else ""
    raise KeyError(f"no plot{where} holds '{vec}' (plots: {[p.plot_name for p in plots]})")


def _xy(raw: Any, vec: str, plot: str | None = None) -> tuple[np.ndarray, np.ndarray]:
    """The plot's sweep axis (column 0) and ``vec``, both real."""
    p = _plot(raw, vec, plot)
    return np.asarray(p[p.variable_names[0]], float), np.asarray(p[vec], float)


# ── spectrum ──

def sndr(raw: Any, vec: str, fs: float, fin: float, n: int, window: str | None = None,
         t0: float | None = None, max_harmonic: int = 6) -> dict[str, Any]:
    """SNDR, SFDR, THD and ENOB of ``n`` samples of ``vec`` taken at ``fs``.

    The samples are ``vec`` linearly interpolated at ``t0 + k/fs``; ``t0=None``
    ends the record at the last time point. Without a window the record must
    be coherent: ``fin*n/fs`` an integer prime to ``n``, so the tone sits in
    one bin and every other bin is noise or distortion. An incoherent record
    raises unless ``window="blackmanharris"`` is named, which counts
    +-5 bins around each tone. THD counts harmonics 2..``max_harmonic``,
    aliased into the first Nyquist zone; a harmonic landing on DC or the
    signal is not counted twice.

    Linear interpolation of a simulator's uneven time points distorts the
    sine; run the transient with a max step well under ``1/fs`` or with
    ngspice's ``.options interp``.
    """
    t, y = _xy(raw, vec)
    return _sndr(t, y, fs, fin, n, window, t0, max_harmonic)


def _sndr(t: np.ndarray, y: np.ndarray, fs: float, fin: float, n: int, window: str | None = None,
          t0: float | None = None, max_harmonic: int = 6) -> dict[str, Any]:
    if n < 8 or not 0 < fin < fs / 2:
        raise ValueError(f"need n >= 8 and 0 < fin < fs/2 (n={n}, fin={fin}, fs={fs})")
    start = t[-1] - n / fs if t0 is None else t0
    if start < t[0] - 1e-9 / fs or start + (n - 1) / fs > t[-1] + 1e-9 / fs:
        raise ValueError(f"record [{t[0]}, {t[-1]}] s does not hold {n} samples from {start} s")
    x = np.interp(start + np.arange(n) / fs, t, y)

    cycles = fin * n / fs
    m = round(cycles)
    coherent = abs(cycles - m) < 1e-6 and math.gcd(m, n) == 1
    if window is None:
        if not coherent:
            raise ValueError(f"incoherent record: fin*n/fs = {cycles:.9g} cycles, needs an integer prime to "
                             f"n={n}; choose fin = fs*M/n with gcd(M, n) = 1 or name window='blackmanharris'")
        w, half = np.ones(n), 0
    elif window == "blackmanharris":
        k = 2 * np.pi * np.arange(n) / n
        a0, a1, a2, a3 = _BLACKMAN_HARRIS
        w, half = a0 - a1 * np.cos(k) + a2 * np.cos(2 * k) - a3 * np.cos(3 * k), _WINDOW_HALF_WIDTH
    else:
        raise ValueError(f"unknown window '{window}' (known: 'blackmanharris')")

    power = np.abs(np.fft.rfft(x * w)) ** 2
    nyq = len(power) - 1
    power[1:nyq if n % 2 == 0 else nyq + 1] *= 2  # one-sided; the Nyquist bin of an even n is not mirrored

    def span(c: int) -> list[int]:
        return list(range(max(c - half, 0), min(c + half, nyq) + 1))

    taken = np.zeros(len(power), bool)
    dc, sig = span(0), span(m)
    if set(dc) & set(sig):
        raise ValueError(f"signal bin {m} lies within {half} bins of DC")
    taken[dc] = taken[sig] = True
    harmonic_bins: dict[int, list[int]] = {}
    for h in range(2, max_harmonic + 1):
        b = (h * m) % n
        bins = [k for k in span(min(b, n - b)) if not taken[k]]
        if bins:
            harmonic_bins[h] = bins
            taken[bins] = True

    p_sig = power[sig].sum()
    p_nd = power.sum() - power[dc].sum() - p_sig
    p_harm = sum(power[b].sum() for b in harmonic_bins.values())
    rest = np.ones(len(power), bool)
    rest[dc] = rest[sig] = False
    spur = power[rest].max() if rest.any() else 0.0
    sndr_db = 10 * math.log10(p_sig / p_nd) if p_nd > 0 else math.inf
    return {
        "sndr_db": sndr_db,
        "sfdr_db": 10 * math.log10(power[sig].max() / spur),
        "enob": (sndr_db - 10 * math.log10(1.5)) / (20 * math.log10(2)),
        "thd_db": 10 * math.log10(p_harm / p_sig) if p_harm > 0 else -math.inf,
        "signal_bin": m,
        "coherent": coherent,
        "signal_bins": sig,
        "harmonic_bins": harmonic_bins,
    }


# ── time domain ──

def settling(raw: Any, vec: str, final: float, tol: float | None = None, t0: float = 0.0, *,
             abs_tol: float | None = None) -> float:
    """Seconds from ``t0`` until ``vec`` enters and stays within a band around ``final``.

    Name one band: ``tol`` is relative to the step, ``|v - final| <=
    tol * |final - v(t0)|``, so a first-order step settles at
    ``tau * ln(1/tol)``; ``abs_tol`` is in the vector's own unit (volts),
    as regulator and start-up specs give it. ``inf`` when the record ends
    outside the band.
    """
    t, y = _xy(raw, vec)
    return _settling(t, y, final, tol, t0, abs_tol=abs_tol)


def _settling(t: np.ndarray, y: np.ndarray, final: float, tol: float | None = None, t0: float = 0.0, *,
              abs_tol: float | None = None) -> float:
    if (tol is None) == (abs_tol is None):
        raise ValueError("name exactly one band: tol (relative to the step) or abs_tol (absolute)")
    if abs_tol is not None:
        band = abs_tol
    else:
        band = tol * abs(final - float(np.interp(t0, t, y)))
        if band <= 0:
            raise ValueError(f"no step: v(t0) equals final ({final}); tol is relative to the step, use abs_tol")
    if band <= 0:
        raise ValueError(f"abs_tol must be positive (got {abs_tol})")
    err = np.abs(y - final)
    out = np.nonzero((err > band) & (t >= t0))[0]
    if len(out) == 0:
        return 0.0
    i = out[-1]
    if i == len(t) - 1:
        return math.inf
    t_in = t[i] + (t[i + 1] - t[i]) * (err[i] - band) / (err[i] - err[i + 1])
    return max(t_in, t0) - t0


def slew(raw: Any, vec: str, lo: float = 0.1, hi: float = 0.9) -> dict[str, float]:
    """Rising and falling slew rate in V/s (both positive) between the ``lo``
    and ``hi`` fractions of the waveform's full swing, from its first rising
    and first falling edge. ``nan`` for an edge the record does not hold.
    """
    t, y = _xy(raw, vec)
    return _slew(t, y, lo, hi)


def _slew(t: np.ndarray, y: np.ndarray, lo: float = 0.1, hi: float = 0.9) -> dict[str, float]:
    # Imported here: testbenches imports remeasure, so a module-level import
    # of the testbenches package from remeasure is circular.
    from .testbenches.analysis import crossings

    if not 0 <= lo < hi <= 1:
        raise ValueError(f"need 0 <= lo < hi <= 1 (lo={lo}, hi={hi})")
    v_lo = y.min() + lo * (y.max() - y.min())
    v_hi = y.min() + hi * (y.max() - y.min())

    def rate(first: float, second: float, rising: bool) -> float:
        for ta in crossings(t, y, first, rising=rising):
            tb = [x for x in crossings(t, y, second, rising=rising) if x > ta]
            if tb:
                return abs(second - first) / (tb[0] - ta)
            break
        return math.nan

    return {"rising": rate(v_lo, v_hi, True), "falling": rate(v_hi, v_lo, False)}


# ── frequency domain ──

def loop_margins(raw: Any, plot: str | None = None, vec: str = "loop_gain") -> dict[str, float]:
    """Phase margin, gain margin and their frequencies from a complex loop gain W.

    ``pm_deg = 180 + angle(W)`` at the first ``|W| = 1`` crossing (``ugf_hz``);
    ``gm_db = -20 log10 |W|`` where W first crosses the negative real axis
    (``pcf_hz``). ``unity_crossings`` counts the ``|W| = 1`` crossings: more
    than one (a conditionally stable or gain-boosted loop) means the first
    margin is not the whole story. Between sweep points ``ln|W|`` and the phase are linear in
    ``ln f``. ``nan`` for a crossing the sweep does not bracket.
    """
    p = _plot(raw, vec, plot)
    return _margins(np.asarray(p[p.variable_names[0]], float), np.asarray(p.complex(vec), complex))


def _margins(f: np.ndarray, w: np.ndarray) -> dict[str, float]:
    lf = np.log(f)
    g = np.log(np.abs(w))
    cross = (g[:-1] * g[1:] <= 0) & (g[:-1] != g[1:])
    out = {"pm_deg": math.nan, "ugf_hz": math.nan, "gm_db": math.nan, "pcf_hz": math.nan,
           "unity_crossings": int(cross.sum())}

    for k in range(1, len(w)):
        if cross[k - 1]:
            frac = g[k - 1] / (g[k - 1] - g[k])
            a0 = np.angle(-w[k - 1])
            a1 = a0 + np.angle(-w[k] / -w[k - 1])  # the short way round
            pm = math.degrees(a0 + frac * (a1 - a0))
            out["pm_deg"] = (pm + 180) % 360 - 180
            out["ugf_hz"] = math.exp(lf[k - 1] + frac * (lf[k] - lf[k - 1]))
            break

    a = np.angle(-w)  # zero on the negative real axis, continuous near it
    for k in range(1, len(w)):
        if w[k - 1].real < 0 and w[k].real < 0 and a[k - 1] * a[k] <= 0 and a[k - 1] != a[k]:
            frac = a[k - 1] / (a[k - 1] - a[k])
            out["gm_db"] = -20 / math.log(10) * (g[k - 1] + frac * (g[k] - g[k - 1]))
            out["pcf_hz"] = math.exp(lf[k - 1] + frac * (lf[k] - lf[k - 1]))
            break
    return out


def noise_total(raw: Any, vec: str = "onoise_spectrum") -> float:
    """Integrated noise in V rms (A rms for a current density) over the swept band.

    ``vec`` is a density in V/sqrt(Hz). Each segment integrates a power law
    through its end points, exact for white, 1/f and 1/f^2 segments and
    close on a log sweep where trapezoids overestimate.
    """
    return _noise_total(*_xy(raw, vec))


def _noise_total(f: np.ndarray, d: np.ndarray) -> float:
    s = np.asarray(d, float) ** 2
    total = 0.0
    for f0, f1, s0, s1 in zip(f[:-1], f[1:], s[:-1], s[1:]):
        if s0 <= 0 or s1 <= 0 or f0 <= 0:
            total += 0.5 * (s0 + s1) * (f1 - f0)
            continue
        r = f1 / f0
        e = math.log(s1 / s0) / math.log(r) + 1
        total += s0 * f0 * (math.log(r) if abs(e) < 1e-12 else (r ** e - 1) / e)
    return math.sqrt(total)


# ── statistics ──

def _betacf(a: float, b: float, x: float) -> float:
    """Continued fraction of the incomplete beta function (modified Lentz)."""
    tiny = 1e-300
    c, d = 1.0, 1.0 - (a + b) * x / (a + 1)
    d = 1 / (d if abs(d) > tiny else tiny)
    h = d
    for m in range(1, 100_000):
        for num in (m * (b - m) * x / ((a + 2 * m - 1) * (a + 2 * m)),
                    -(a + m) * (a + b + m) * x / ((a + 2 * m) * (a + 2 * m + 1))):
            d = 1 + num * d
            d = 1 / (d if abs(d) > tiny else tiny)
            c = 1 + num / c
            c = c if abs(c) > tiny else tiny
            h *= d * c
        if abs(d * c - 1) < 1e-15:
            return h
    raise ArithmeticError(f"incomplete beta did not converge (a={a}, b={b}, x={x})")


def _betainc(a: float, b: float, x: float) -> float:
    """Regularized incomplete beta I_x(a, b)."""
    if x <= 0:
        return 0.0
    if x >= 1:
        return 1.0
    front = math.exp(math.lgamma(a + b) - math.lgamma(a) - math.lgamma(b) + a * math.log(x) + b * math.log1p(-x))
    if x < (a + 1) / (a + b + 2):
        return front * _betacf(a, b, x) / a
    return 1 - front * _betacf(b, a, 1 - x) / b


def _root(f: Any) -> float:
    """The p in [0, 1] where increasing ``f`` crosses zero, by bisection."""
    lo, hi = 0.0, 1.0
    for _ in range(200):
        mid = 0.5 * (lo + hi)
        if mid in (lo, hi):
            break
        lo, hi = (mid, hi) if f(mid) < 0 else (lo, mid)
    return 0.5 * (lo + hi)


def yield_ci(passes: int, n: int, conf: float = 0.95) -> tuple[float, float]:
    """Clopper-Pearson (exact binomial) two-sided interval on the pass rate.

    2300 passes of 2300 at 95% gives (0.99840, 1.0): a clean Monte Carlo
    proves the yield only down to the low end.
    """
    if n <= 0 or not 0 <= passes <= n or not 0 < conf < 1:
        raise ValueError(f"need 0 <= passes <= n, n > 0, 0 < conf < 1 (passes={passes}, n={n}, conf={conf})")
    alpha = 1 - conf
    k = passes
    low = 0.0 if k == 0 else _root(lambda p: _betainc(k, n - k + 1, p) - alpha / 2)
    high = 1.0 if k == n else _root(lambda p: _betainc(k + 1, n - k, p) - (1 - alpha / 2))
    return low, high


# ── evidence ──

def compare(meas_json: str | Path | Mapping[str, Any], recomputed: Mapping[str, float],
            rtol: float = 1e-3, atol: float = 0.0) -> list[dict[str, Any]]:
    """Every disagreement between a simulator's measures and recomputed values.

    ``meas_json`` is ESPice's ``--measures`` output (``espice-measures/1``), as
    a path or the loaded dict; its nominal-variant results are matched to
    ``recomputed`` by name, case-insensitive. A name the simulator did not
    report, a failed measure and a value outside ``rtol``/``atol`` each give
    one row. An empty list is agreement; the list is the gate's evidence.
    """
    data = meas_json if isinstance(meas_json, Mapping) else json.loads(Path(meas_json).read_text())
    if data.get("format") != "espice-measures/1":
        raise ValueError(f"not an espice-measures/1 document (format={data.get('format')!r})")
    sim = {r["name"].lower(): r for r in data["results"] if r.get("variant", {}).get("kind") == "nominal"}
    rows = []
    for name, value in recomputed.items():
        r = sim.get(name.lower())
        theirs = None if r is None else r.get("value")
        if r is None:
            reason = "simulator reported no nominal measure of this name"
        elif theirs is None:
            reason = f"simulator measure failed: {r.get('error', 'no value')}"
        elif not math.isclose(theirs, value, rel_tol=rtol, abs_tol=atol):
            reason = f"differs by {abs(theirs - value):.6g} (rtol={rtol}, atol={atol})"
        else:
            continue
        rows.append({"name": name, "simulator": theirs, "recomputed": value, "reason": reason})
    return rows
