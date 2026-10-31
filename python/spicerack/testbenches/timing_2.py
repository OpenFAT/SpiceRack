"""PLL and DLL benches: lock time, loop margins, reference spur, PFD/CP transfer, DLL lock.

Lock, spur and PFD benches run a transient on the whole loop and read edges. The
loop margins come from a linear phase-domain AC model built around the caller's
loop filter, because a PFD/CP has no operating point to linearize a transistor
loop about (pstb is Spectre-only). Gains are in Hz and cycles: ``ip`` in A per
cycle of phase error, ``kvco_hz_per_v`` in Hz/V; the 2*pi factors cancel.
"""

from __future__ import annotations

import json
import math
import warnings
from typing import Any

import numpy as np

from ..remeasure import _margins, _settling
from ._common import ac_complex
from .analysis import HALF_POWER_DB, crossings
from .design import DesignBench
from .timing import _clock, _edges, _tone, _tran


def _mid(v: np.ndarray, w: np.ndarray) -> float:
    return 0.5 * float(v[w].max() + v[w].min())


def pll_lock_time(
    ps: Any,
    dut: Any,
    *,
    reference_node: str = "ref",
    output_node: str = "vco",
    control_node: str = "vctrl",
    feedback_node: str | None = None,
    ground: str = "0",
    reference_frequency: float = 10e6,
    divide_ratio: float = 1.0,
    step_ratio: float = 1e-2,
    pre_cycles: int = 200,
    post_cycles: int = 600,
    tolerance: float = 0.05,
    points_per_period: int = 500,
    v_high: float = 1.0,
) -> DesignBench:
    """Frequency (and phase) settling after a reference frequency step.

    The reference runs at f1 for ``pre_cycles`` (lock), then steps phase-continuously
    to f2 = f1 * (1 + ``step_ratio``) at t_step. Output frequency is averaged over one
    reference period (N VCO periods), which removes the reference-rate CP ripple.
    ``lock_time_s``: from t_step until it stays within ``tolerance`` of the step
    N*(f2 - f1) around N*f2. Raises when the pre-step or final residual exceeds a third
    of that band: either the loop never locked, or the tolerance sits below the ripple
    floor and the number would measure ripple. With ``feedback_node`` (divider output):
    the phase error e_k = t_fb - t_ref, its final value and its peak excursion after
    the step; a cycle slip (|de| > T_ref/2) raises. An XSPICE PFD sees an edge at the
    next time point, so each CP pulse is quantised to the step h, which kicks the
    averaged frequency by Kv*ip*R*h*f_ref through the filter zero R: for a digital
    PFD raise ``points_per_period`` until that is well inside the band.
    """
    f1, f2 = reference_frequency, reference_frequency * (1 + step_ratio)
    t_step = pre_cycles / f1
    end = t_step + post_cycles / f2
    edge = 1 / (100 * max(f1, f2))
    rises = np.concatenate([np.arange(pre_cycles) / f1, t_step + np.arange(post_cycles) / f2])
    highs = np.concatenate([np.full(pre_cycles, 0.5 / f1), np.full(post_cycles, 0.5 / f2)])
    tb = ps.Testbench(dut)
    _clock(tb, "ref", reference_node, ground, rises, highs, edge, v_high)
    tb.save(f"V({output_node})", f"V({control_node})", *([f"V({feedback_node})"] if feedback_node else []))
    f_max = divide_ratio * max(f1, f2)
    runner = _tran(tb, step_time=1 / (2 * f_max * points_per_period), end_time=end,
                   max_time=1 / (f_max * points_per_period))

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        v = np.asarray(tran[output_node], float)
        tail = t >= t_step + 0.75 * (end - t_step)
        e = _edges(t, v, _mid(v, tail))
        n = max(1, round(divide_ratio))
        fbar, tm = n / (e[n:] - e[:-n]), 0.5 * (e[n:] + e[:-n])
        n1, n2 = divide_ratio * f1, divide_ratio * f2
        band = tolerance * abs(n2 - n1)
        pre = (tm > 0.75 * t_step) & (tm < t_step - 1 / f1)
        post = tm >= t_step + 0.75 * (end - t_step)
        if not pre.any() or np.abs(fbar[pre] - n1).max() > band / 3:
            raise ValueError(f"not locked before the step: residual over {band / 3:g} Hz; raise pre_cycles")
        if np.abs(fbar[post] - n2).max() > band / 3:
            raise ValueError(f"final residual {np.abs(fbar[post] - n2).max():g} Hz exceeds tolerance/3 = "
                             f"{band / 3:g} Hz: the band is below the ripple floor (raise points_per_period or "
                             "tolerance), or the loop did not settle")
        # The average whose window ends at t_step is the last pre-step value.
        t0 = t_step - 0.5 / f1
        y0 = float(np.interp(t0, tm, fbar))
        lock = t0 + _settling(tm, fbar, n2, band / abs(n2 - y0), t0) - t_step
        after = tm > t_step
        ctl = np.asarray(tran[control_node], float)[tail]
        m = {
            "lock_time_s": float(lock),
            "frequency_overshoot_ratio": float(max(0.0, (np.sign(n2 - n1) * (fbar[after] - n2)).max()) / abs(n2 - n1)),
            "control_final_v": float(ctl.mean()),
            "control_ripple_v": float(np.ptp(ctl)),
        }
        if feedback_node is not None:
            fb_v = np.asarray(tran[feedback_node], float)
            fb = _edges(t, fb_v, _mid(fb_v, tail))
            fb = fb[fb > 0.75 * t_step]  # from the locked pre-step window: acquisition may slip
            ref = rises + edge / 2
            k = np.clip(np.searchsorted(ref, fb), 1, len(ref) - 1)
            k -= (fb - ref[k - 1]) < (ref[k] - fb)
            err = fb - ref[k]
            if np.any(np.abs(np.diff(err)) > 0.5 / f1):
                raise ValueError("cycle slip: the step exceeds the pull-out range; phase metrics undefined")
            final = float(err[fb >= t_step + 0.75 * (end - t_step)].mean())
            m["static_phase_error_s"] = final
            m["phase_error_peak_s"] = float(np.abs(err[fb > t_step] - final).max())
        return m

    phase = ["static_phase_error_s", "phase_error_peak_s"] if feedback_node else []
    return DesignBench(
        name="pll_lock_time",
        category="pll",
        testbench=tb,
        intent=f"Settling to {tolerance:g} of a reference frequency step, overshoot and phase error.",
        measurements=["lock_time_s", "frequency_overshoot_ratio", "control_final_v", "control_ripple_v"] + phase,
        result_fields=["tran.time", f"tran['{output_node}']", f"tran['{control_node}']"],
        validation=["locks within budget", "overshoot in budget"],
        extractor=extract,
        runner=runner,
    )


def pll_loop_margins(
    ps: Any,
    dut: Any,
    *,
    ip: float,
    kvco_hz_per_v: float,
    filter_in: str = "cp",
    filter_out: str = "vctrl",
    divide_ratio: float = 1.0,
    reference_frequency: float = 10e6,
    points_per_decade: int = 100,
) -> DesignBench:
    """Open-loop crossover, phase margin and closed-loop bandwidth/peaking of a CP-PLL.

    The DUT is the loop-filter subcircuit (CP output ``filter_in``, VCO control
    ``filter_out``, which may be the same port). The bench closes a linear
    phase-domain loop around it: PFD/CP ``ip`` A per cycle, VCO ``kvco_hz_per_v``,
    divider 1/N. The loop is broken at the PFD input, so L = theta_fb / theta_err;
    the closed loop is H/N = L/(1 + L), exact for this model. The continuous-time
    model holds only for crossover << f_ref: ``crossover_over_ref`` above 0.1 warns
    (Gardner 1980). Swept from f_ref/1e5 to f_ref.
    """
    spec = json.loads(dut.to_json())
    vctl = "cp" if filter_out == filter_in else "vctl"
    ports = {filter_in: "cp", filter_out: vctl}
    loop = ps.Subcircuit("pll_phase_model", [])
    loop.X("filt", spec["name"], *[ports.get(p["name"], p["name"]) for p in spec["ports"]])
    loop.G(name="cp", positive="0", negative="cp", control_positive="err", control_negative="0", transconductance=ip)
    loop.G(name="vco", positive="0", negative="ph", control_positive=vctl, control_negative="0",
           transconductance=kvco_hz_per_v)
    loop.C(name="vco", positive="ph", negative="0", value=1.0)
    loop.E(name="div", positive="fb", negative="0", control_positive="ph", control_negative="0",
           voltage_gain=1.0 / divide_ratio)
    # DC paths for the operating point; their poles sit below 1e-6 Hz.
    loop.R(name="cpdc", positive="cp", negative="0", value=1e15)
    loop.R(name="vcodc", positive="ph", negative="0", value=1e15)
    tb = ps.Testbench(loop)
    tb.add_subcircuit(dut)
    tb.V(name="err", positive="err", negative="0", value=0.0, ac=1.0)
    tb.save("V(fb)")
    decades = 5
    ac_args = dict(variation="dec", number_of_points=points_per_decade,
                   start_frequency=reference_frequency / 10 ** decades, stop_frequency=reference_frequency)
    tb.add_ac(**ac_args)

    def extract(ac: Any) -> dict[str, float]:
        f = np.asarray(ac.frequency, float)
        w = ac_complex(ac, "fb")
        mg = _margins(f, w)
        if math.isnan(mg["ugf_hz"]):
            raise ValueError("|L| never crosses 1 between f_ref/1e5 and f_ref")
        db = 20 * np.log10(np.abs(w / (1 + w)))
        bw = crossings(f, db, db[0] - HALF_POWER_DB, rising=False, log_x=True)
        if not bw:
            raise ValueError("closed loop never falls 3 dB below its low-frequency gain below f_ref")
        ratio = mg["ugf_hz"] / reference_frequency
        if ratio > 0.1:
            warnings.warn(f"crossover at {ratio:.3g} f_ref: the continuous-time model is unreliable above 0.1")
        return {
            "crossover_hz": mg["ugf_hz"],
            "phase_margin_deg": mg["pm_deg"],
            "closed_loop_bw_hz": bw[0],
            "peaking_db": float(db.max() - db[0]),
            "crossover_over_ref": ratio,
        }

    return DesignBench(
        name="pll_loop_margins",
        category="pll",
        testbench=tb,
        intent="Loop-filter design check: crossover, phase margin, closed-loop bandwidth and peaking.",
        measurements=["crossover_hz", "phase_margin_deg", "closed_loop_bw_hz", "peaking_db", "crossover_over_ref"],
        result_fields=["ac.frequency", "ac['fb']"],
        validation=["phase margin above target", "crossover well below f_ref"],
        extractor=extract,
        runner=lambda: tb.ac(**ac_args),
    )


def pll_reference_spur(
    ps: Any,
    dut: Any,
    *,
    reference_node: str = "ref",
    output_node: str = "vco",
    control_node: str = "vctrl",
    ground: str = "0",
    reference_frequency: float = 10e6,
    divide_ratio: float = 1.0,
    lock_cycles: int = 200,
    measure_cycles: int = 256,
    harmonics: int = 3,
    points_per_period: int = 200,
    v_high: float = 1.0,
) -> DesignBench:
    """Deterministic reference spurs of a locked PLL, in dBc, from the output edges.

    Spurs need no noise: a noiseless locked transient holds them. Over the last
    ``measure_cycles`` reference periods the VCO rising edges give the time-interval
    error against their least-squares line, phase phi_k = 2*pi*TIE_k/T. The spur at
    h*f_ref is 20*log10(beta_h/2), beta_h the least-squares phase amplitude at h*f_ref
    (narrowband FM: exact to beta**2/8). No FFT, window or resampling. The edges sample
    the phase at N*f_ref, so h*f_ref is observable only for N > 2h: the bench refuses
    ``divide_ratio <= 2 * harmonics`` (at N = 1 the f_ref spur aliases to DC).
    """
    if divide_ratio <= 2 * harmonics:
        raise ValueError(f"output edges sample the phase at {divide_ratio:g}*f_ref: spurs up to {harmonics}*f_ref "
                         f"need divide_ratio > {2 * harmonics}; lower harmonics or measure a faster node")
    period = 1.0 / reference_frequency
    end = (lock_cycles + measure_cycles) * period
    tb = ps.Testbench(dut)
    _clock(tb, "ref", reference_node, ground, period * np.arange(lock_cycles + measure_cycles), period / 2,
           period / 100, v_high)
    tb.save(f"V({output_node})", f"V({control_node})")
    f_out = divide_ratio * reference_frequency
    runner = _tran(tb, step_time=1 / (2 * f_out * points_per_period), end_time=end,
                   max_time=1 / (f_out * points_per_period))

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        v = np.asarray(tran[output_node], float)
        w = t >= lock_cycles * period
        e = _edges(t, v, _mid(v, w))
        e = e[e >= lock_cycles * period]
        k = np.arange(len(e))
        slope, icpt = np.polyfit(k, e, 1)
        phi = 2 * np.pi * (e - (icpt + slope * k)) / slope
        ctl = np.asarray(tran[control_node], float)[w]
        m = {"control_ripple_v": float(np.ptp(ctl))}
        for h in range(1, harmonics + 1):
            m["reference_spur_dbc" if h == 1 else f"reference_spur_h{h}_dbc"] = (
                20 * math.log10(_tone(e, phi, h * reference_frequency) / 2))
        return m

    return DesignBench(
        name="pll_reference_spur",
        category="pll",
        testbench=tb,
        intent="Reference spurs (CP mismatch, leakage, charge sharing) of a locked PLL.",
        measurements=["reference_spur_dbc"] + [f"reference_spur_h{h}_dbc" for h in range(2, harmonics + 1)]
        + ["control_ripple_v"],
        result_fields=["tran.time", f"tran['{output_node}']", f"tran['{control_node}']"],
        validation=["reference spur below target"],
        extractor=extract,
        runner=runner,
    )


def _mean(t: np.ndarray, y: np.ndarray, a: float, b: float) -> float:
    """Time average of ``y`` over [a, b] on the native grid (trapezoids)."""
    ts = np.concatenate([[a], t[(t > a) & (t < b)], [b]])
    return float(np.trapezoid(np.interp(ts, t, y), ts) / (b - a))


def pfd_cp_transfer(
    ps: Any,
    dut: Any,
    *,
    reference_node: str = "ref",
    feedback_node: str = "fb",
    output_node: str = "cp",
    ground: str = "0",
    reference_frequency: float = 10e6,
    skews: list[float] | None = None,
    output_voltage: float = 0.5,
    cycles_per_step: int = 8,
    fit_from: float | None = None,
    edge: float | None = None,
    points_per_period: int = 200,
    v_high: float = 1.0,
) -> DesignBench:
    """Average PFD/CP output current against phase offset: gain, offset, dead zone.

    ``feedback_node`` lags ``reference_node`` by each skew in ``skews`` (s, positive =
    feedback late) for ``cycles_per_step`` cycles; ``output_node`` is held at
    ``output_voltage`` by the ammeter source. Each step's mean current comes from its
    last half. Lines are fitted to the current against phase (rad) on both sides,
    |skew| >= ``fit_from`` (default max|skew|/4): ``kpd_a_per_rad`` is their mean
    slope (ideal Ip/2pi), ``dead_zone_s`` the gap between their zero crossings (a CP
    mismatch d adds d**2*t_rst/(1+d) to it), ``static_offset_s`` the skew where the
    measured mean current crosses zero (where a PLL locks; the middle crossing inside a
    dead zone), ``linearity_pct`` the worst residual over kpd * phi_max. Clock ramps
    are ``edge`` (default T/100) long; the PFD sees an edge anywhere inside its ramp, so
    offsets resolve only to ~``edge``. Near zero skew the UP and DN pulses overlap and
    the mean current is only as good as the time grid on their corners: raise
    ``points_per_period`` for ps offsets.
    """
    period = 1.0 / reference_frequency
    if skews is None:
        skews = (period / 20) * np.array([-1, -.5, -.2, -.1, -.05, -.02, -.01, 0, .01, .02, .05, .1, .2, .5, 1])
    skews = np.asarray(skews, float)
    edge = period / 100 if edge is None else edge
    n = len(skews) * cycles_per_step
    rises = period / 2 + period * np.arange(n)
    tb = ps.Testbench(dut)
    _clock(tb, "ref", reference_node, ground, rises, period / 4, edge, v_high)
    _clock(tb, "fb", feedback_node, ground, rises + np.repeat(skews, cycles_per_step), period / 4, edge, v_high)
    tb.V(name="cpout", positive=output_node, negative=ground, value=output_voltage)
    tb.save("I(Vcpout)")
    runner = _tran(tb, step_time=period / (2 * points_per_period), end_time=n * period,
                   max_time=period / points_per_period)

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        i = np.asarray(tran["i(vcpout)"], float)
        hold = cycles_per_step * period
        avg = np.array([_mean(t, i, (j + 1) * hold - (cycles_per_step // 2) * period, (j + 1) * hold)
                        for j in range(len(skews))])
        phi = 2 * np.pi * skews / period
        lo = (fit_from if fit_from is not None else np.abs(skews).max() / 4) * 2 * np.pi / period
        fits = [np.polyfit(phi[s], avg[s], 1) for s in (phi >= lo, phi <= -lo)]
        if any(f[0] <= 0 for f in fits):
            raise ValueError("current does not rise with feedback lag on both sides; check UP/DN polarity")
        x_pos, x_neg = (-f[1] / f[0] * period / (2 * np.pi) for f in fits)
        kpd = 0.5 * (fits[0][0] + fits[1][0])
        zero = crossings(skews, avg, 0.0)
        if not zero:
            raise ValueError("mean CP current never crosses zero over the skews")
        resid = max(np.abs(avg[s] - np.polyval(f, phi[s])).max() for f, s in zip(fits, (phi >= lo, phi <= -lo)))
        return {
            "kpd_a_per_rad": float(kpd),
            "static_offset_s": float(zero[len(zero) // 2]),
            "dead_zone_s": float(max(0.0, x_pos - x_neg)),
            "linearity_pct": float(100 * resid / (kpd * np.abs(phi).max())),
        }

    return DesignBench(
        name="pfd_cp_transfer",
        category="pll",
        testbench=tb,
        intent="PFD/charge-pump gain, static phase offset, dead zone and linearity.",
        measurements=["kpd_a_per_rad", "static_offset_s", "dead_zone_s", "linearity_pct"],
        result_fields=["tran.time", "tran['i(vcpout)']"],
        validation=["gain near Ip/2pi", "no dead zone", "offset in budget"],
        extractor=extract,
        runner=runner,
    )


def dll_lock(
    ps: Any,
    dut: Any,
    *,
    reference_node: str = "ref",
    output_node: str = "out",
    control_node: str = "vctrl",
    ground: str = "0",
    reference_frequency: float = 10e6,
    cycles: int = 120,
    control_initial: float | None = None,
    tolerance_s: float | None = None,
    points_per_period: int = 1000,
    v_high: float = 1.0,
) -> DesignBench:
    """Lock of a delay-locked loop: phase error, locked harmonic, lock cycles.

    A reference clock (first edge at t=0) drives the DUT; ``control_initial`` sets the
    control node's starting voltage. Output edge k is paired with reference edge k,
    d_k the delay; the loop locks on n = round(d/T) over the last 25 %
    (``locked_harmonic``; n != 1 is the false lock, reported, not raised), and
    e_k = d_k - n*T. ``lock_cycles``: first k after which |e_k - e_final| stays below
    ``tolerance_s``, default 10 max steps (T/100): an XSPICE PFD sees edges at the next
    time point, so below ~1 step the count measures quantisation, not settling.
    """
    period = 1.0 / reference_frequency
    tol = 10 * period / points_per_period if tolerance_s is None else tolerance_s
    tb = ps.Testbench(dut)
    _clock(tb, "ref", reference_node, ground, period * np.arange(cycles), period / 2, period / 100, v_high)
    if control_initial is not None:
        tb.initial_condition(**{control_node: control_initial})
    tb.save(f"V({output_node})", f"V({control_node})")
    runner = _tran(tb, step_time=period / (2 * points_per_period), end_time=cycles * period,
                   max_time=period / points_per_period)

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        ref = period * np.arange(cycles) + period / 200
        out = _edges(t, np.asarray(tran[output_node], float), v_high / 2)
        out = out[out > ref[0]]
        d = out - ref[:len(out)]
        tail = slice(3 * len(d) // 4, None)
        n = round(float(d[tail].mean()) / period)
        e = d - n * period
        final = float(e[tail].mean())
        late = np.nonzero(np.abs(e - final) >= tol)[0]
        ctl = np.asarray(tran[control_node], float)
        return {
            "static_phase_error_s": final,
            "locked_harmonic": float(n),
            "lock_cycles": float(late[-1] + 1 if late.size else 0),
            "control_final_v": float(ctl[t >= 0.75 * t[-1]].mean()),
        }

    return DesignBench(
        name="dll_lock",
        category="dll",
        testbench=tb,
        intent="DLL lock: static phase error, locked harmonic (false lock) and cycles to lock.",
        measurements=["static_phase_error_s", "locked_harmonic", "lock_cycles", "control_final_v"],
        result_fields=["tran.time", f"tran['{output_node}']", f"tran['{control_node}']"],
        validation=["locks on n = 1", "static phase error in budget", "locks within budget"],
        extractor=extract,
        runner=runner,
    )
