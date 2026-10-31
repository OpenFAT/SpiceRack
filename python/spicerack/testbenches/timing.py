"""Timing and frequency benches: oscillators, VCOs, clock buffers, delay lines, crystal cores.

Every transient bench measures edges: threshold crossings linearly interpolated on
the simulator's own time points. Accuracy then follows the integrator, not the
extractor. A trapezoidal transient shifts a sinusoidal tank by -(2*pi*h/T)**2/12
(h the step), so ``points_per_period`` (max step = T / it) is the accuracy knob:
-82 ppm at the default 200. Event-driven oscillators (relaxation, ring) err
linearly in h instead. Gear integration damps a tank and is never used here.
"""

from __future__ import annotations

import math
from typing import Any

import numpy as np

from ._common import ac_complex, spot
from .analysis import crossings
from .design import DesignBench, _step_pwl


# ── shared edge helpers (timing_2 uses them too) ──

def _edges(t: np.ndarray, v: np.ndarray, threshold: float, rising: bool = True,
           hysteresis: float = 0.0) -> np.ndarray:
    """Crossings of ``threshold`` in one direction. With ``hysteresis`` an edge counts
    only once the signal has gone back past ``threshold -/+ hysteresis`` since the last
    one: a slow, noisy edge crosses several times and each would be a fake period."""
    e = np.asarray(crossings(t, v, threshold, rising=rising))
    if hysteresis <= 0 or len(e) < 2:
        return e
    rearm = crossings(t, v, threshold - hysteresis if rising else threshold + hysteresis, rising=not rising)
    armed = np.searchsorted(np.asarray(rearm), e)
    keep = [0]
    for k in range(1, len(e)):
        if armed[k] > armed[keep[-1]]:
            keep.append(k)
    return e[keep]


def _clock(tb: Any, name: str, node: str, ground: str, rises: Any, high: Any, edge: float,
           level: float = 1.0) -> None:
    """PWL clock: from each time in ``rises`` a pulse high for ``high`` (50 %-to-50 %),
    with linear ``edge`` ramps. Explicit edges, so a skewed or frequency-stepped clock
    stays phase continuous."""
    rises = np.asarray(rises, float)
    high = np.broadcast_to(np.asarray(high, float), rises.shape)
    assert np.all(high > edge) and np.all(np.diff(rises) > high[:-1] + edge), "pulses overlap"
    points = [] if rises[0] == 0 else [(0.0, 0.0)]
    for r, h in zip(rises.tolist(), high.tolist()):
        points += [(r, 0.0), (r + edge, level), (r + h, level), (r + h + edge, 0.0)]
    tb.PieceWiseLinearVoltageSource(name=name, positive=node, negative=ground, values=points)


def _tone(t: np.ndarray, y: np.ndarray, f: float) -> float:
    """Amplitude of the ``f`` sinusoid in samples ``y`` taken at times ``t``.

    Least squares on [1, sin, cos]: exact for any sample spacing and any window
    length, so the record needs no coherence and no resampling."""
    w = 2 * np.pi * f * np.asarray(t, float)
    a = np.column_stack([np.ones_like(w), np.sin(w), np.cos(w)])
    coef = np.linalg.lstsq(a, np.asarray(y, float), rcond=None)[0]
    return float(math.hypot(coef[1], coef[2]))


def _slopes(x: Any, y: Any) -> tuple[float, np.ndarray]:
    """Central-difference slope at the middle of ``x``, and every chord slope."""
    x, y = np.asarray(x, float), np.asarray(y, float)
    assert len(x) >= 2 and np.all(np.diff(x) > 0), "sweep values must rise"
    m = (len(x) - 1) / 2
    i, j = math.ceil(m) - 1, math.floor(m) + 1
    return float((y[j] - y[i]) / (x[j] - x[i])), np.diff(y) / np.diff(x)


def _window_frequency(t: np.ndarray, v: np.ndarray, t0: float, t1: float,
                      threshold: float | None, what: str) -> float:
    """Mean frequency of the rising edges in [t0, t1]: (K - 1) / (e_K - e_1)."""
    w = (t >= t0) & (t <= t1)
    thr = 0.5 * (v[w].max() + v[w].min()) if threshold is None else threshold
    e = _edges(t, v, thr)
    e = e[(e >= t0) & (e <= t1)]
    if len(e) < 3:
        raise ValueError(f"no oscillation {what}: {len(e)} rising edges through {thr:g} V in [{t0:g}, {t1:g}] s")
    return float((len(e) - 1) / (e[-1] - e[0]))


def _signal(res: Any, node: str, node_b: str | None) -> tuple[np.ndarray, np.ndarray]:
    t = np.asarray(res.time, float)
    v = np.asarray(res[node], float)
    return t, v if node_b is None else v - np.asarray(res[node_b], float)


def _tran(tb: Any, **tran: float) -> Any:
    """Queue the transient on ``tb`` and return the runner that runs the same one."""
    tb.add_transient(**tran)
    return lambda: tb.transient(**tran)


# ── oscillators ──

def _oscillation(t: np.ndarray, v: np.ndarray, t_settle: float, threshold: float | None,
                 t_start: float) -> dict[str, float]:
    w = t >= t_settle
    thr = 0.5 * (v[w].max() + v[w].min()) if threshold is None else threshold
    rise, fall = _edges(t, v, thr), _edges(t, v, thr, rising=False)
    settled = rise[rise >= t_settle]
    if len(settled) < 3:
        raise ValueError(f"no oscillation after {t_settle:g} s: {len(settled)} rising edges through "
                         f"{thr:g} V (latched at {v[-1]:g} V?)")
    period = (settled[-1] - settled[0]) / (len(settled) - 1)
    # Per cycle [rise_k, rise_k+1): max - min. ponytail: raw sample extremes read
    # low by (2*pi/points_per_period)**2/8, 1.2e-4 at 200; parabolic refinement if that matters.
    idx = np.searchsorted(t, rise)
    amps = (np.maximum.reduceat(v, idx) - np.minimum.reduceat(v, idx))[:-1]
    amp = float(amps[rise[:-1] >= t_settle].mean())
    high = fall[np.searchsorted(fall, settled[:-1])] - settled[:-1]
    low = np.nonzero(amps < 0.9 * amp)[0]
    t_up = rise[low[-1] + 1] if low.size else rise[0]
    return {
        "frequency_hz": float(1.0 / period),
        "period_spread_s": float(np.ptp(np.diff(settled))),
        "amplitude_pp_v": amp,
        "duty_cycle": float(high.mean() / period),
        "startup_time_s": float(max(t_up - t_start, 0.0)),
        "window": (float(settled[0]), float(settled[-1])),
    }


def oscillator_frequency(
    ps: Any,
    dut: Any,
    *,
    expected_frequency: float,
    output_node: str = "out",
    output_node_b: str | None = None,
    supply_node: str | None = None,
    vdd: float = 1.8,
    supply_ramp: float = 0.0,
    kick_node: str | None = None,
    kick_charge: float = 1e-14,
    ground: str = "0",
    cycles: int = 200,
    settle_fraction: float = 0.5,
    threshold: float | None = None,
    points_per_period: int = 200,
) -> DesignBench:
    """Free-running frequency, swing, duty cycle and startup of an autonomous oscillator.

    Measured on ``V(output_node) - V(output_node_b)`` (single-ended without ``_b``).
    Startup is deterministic, never left to numerical noise: ``kick_node`` gets
    ``kick_charge`` in a T/20 current pulse at t=0, and/or ``supply_node`` ramps from
    0 to ``vdd`` over ``supply_ramp`` seconds (0: a DC supply). ``startup_time_s`` runs
    from the later of the two to the first rising edge after which every cycle's swing
    stays at 90 % of the settled swing. Frequency, swing and duty come from the last
    ``1 - settle_fraction`` of the record, the threshold from its mid-swing unless
    given. A latched oscillator raises rather than reporting 0 Hz.
    """
    period = 1.0 / expected_frequency
    tb = ps.Testbench(dut)
    t_start = 0.0
    signals = [f"V({output_node})"] + ([f"V({output_node_b})"] if output_node_b else [])
    if supply_node is not None:
        if supply_ramp > 0:
            tb.PieceWiseLinearVoltageSource(name="dd", positive=supply_node, negative=ground,
                                            values=[(0.0, 0.0), (supply_ramp, vdd)])
        else:
            tb.V(name="dd", positive=supply_node, negative=ground, value=vdd)
        signals.append("I(Vdd)")
        t_start = supply_ramp
    if kick_node is not None:
        edge, width = period / 200, period / 20
        # A trapezoid holds amplitude * (width + edge) of charge.
        amp = kick_charge / (width + edge)
        tb.PieceWiseLinearCurrentSource(name="kick", positive=ground, negative=kick_node, values=[
            (0.0, 0.0), (edge, amp), (edge + width, amp), (2 * edge + width, 0.0)])
        t_start = max(t_start, 2 * edge + width)
    tb.save(*signals)
    end = t_start + cycles * period
    runner = _tran(tb, step_time=period / (2 * points_per_period), end_time=end,
                   max_time=period / points_per_period)

    def extract(tran: Any) -> dict[str, float]:
        t, v = _signal(tran, output_node, output_node_b)
        m = _oscillation(t, v, settle_fraction * t[-1], threshold, t_start)
        a, b = m.pop("window")
        if supply_node is not None:
            w = (t >= a) & (t <= b)
            m["supply_current_a"] = float(-np.trapezoid(np.asarray(tran["i(vdd)"], float)[w], t[w]) / (t[w][-1] - t[w][0]))
        return m

    measurements = ["frequency_hz", "period_spread_s", "amplitude_pp_v", "duty_cycle", "startup_time_s"]
    return DesignBench(
        name="oscillator_frequency",
        category="oscillator",
        testbench=tb,
        intent="Free-running frequency, swing, duty cycle and startup time of an oscillator.",
        measurements=measurements + (["supply_current_a"] if supply_node is not None else []),
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["oscillates and settles", "frequency in range", "starts within budget"],
        extractor=extract,
        runner=runner,
    )


def oscillator_sweep(
    ps: Any,
    dut: Any,
    *,
    values: list[float],
    frequency_range: tuple[float, float],
    output_node: str = "out",
    sweep_node: str = "vctrl",
    fixed: dict[str, float] | None = None,
    ground: str = "0",
    settle_cycles: int = 50,
    measure_cycles: int = 100,
    threshold: float | None = None,
    points_per_period: int = 200,
) -> DesignBench:
    """Frequency against a DC voltage on ``sweep_node``: KVCO and tuning range when it
    is the control, supply pushing when it is the supply.

    One transient with a staircase on ``sweep_node`` (one level per ``values`` entry,
    each held ``settle_cycles + measure_cycles`` periods of ``frequency_range[0]``);
    ``fixed`` holds the other pins. ``df_dv_hz_per_v`` is the central-difference
    slope at the middle value, in Hz/V (multiply by 2*pi for rad/s/V);
    ``df_dv_max_over_min`` the spread of the chord slopes. A level that does not
    oscillate raises, naming the value.
    """
    f_lo, f_hi = frequency_range
    hold = (settle_cycles + measure_cycles) / f_lo
    tb = ps.Testbench(dut)
    for node, volts in (fixed or {}).items():
        tb.V(name=f"fix_{node}", positive=node, negative=ground, value=volts)
    _step_pwl(tb, "sweep", sweep_node, ground, values, hold)
    tb.save(f"V({output_node})", f"V({sweep_node})")
    runner = _tran(tb, step_time=1 / (2 * f_hi * points_per_period), end_time=len(values) * hold,
                   max_time=1 / (f_hi * points_per_period))

    def extract(tran: Any) -> dict[str, float]:
        t, v = _signal(tran, output_node, None)
        f = np.array([_window_frequency(t, v, (k + 1) * hold - measure_cycles / f_lo, (k + 1) * hold,
                                        threshold, f"at {sweep_node} = {x:g} V")
                      for k, x in enumerate(values)])
        mid, chords = _slopes(values, f)
        m = {f"frequency_hz_{k}": float(x) for k, x in enumerate(f)}
        m.update({
            "f_min_hz": float(f.min()),
            "f_max_hz": float(f.max()),
            "tuning_range_ratio": float(2 * (f.max() - f.min()) / (f.max() + f.min())),
            "df_dv_hz_per_v": mid,
            "df_dv_max_over_min": float(np.abs(chords).max() / np.abs(chords).min()),
            "monotonic": float(np.all(chords > 0) or np.all(chords < 0)),
        })
        return m

    return DesignBench(
        name="oscillator_sweep",
        category="vco",
        testbench=tb,
        intent="Frequency against a swept pin: KVCO, tuning range and linearity, or supply pushing.",
        measurements=[f"frequency_hz_{k}" for k in range(len(values))] + [
            "f_min_hz", "f_max_hz", "tuning_range_ratio", "df_dv_hz_per_v", "df_dv_max_over_min", "monotonic"],
        result_fields=["tran.time", f"tran['{output_node}']", f"tran['{sweep_node}']"],
        validation=["monotonic tuning", "range covers target", "KVCO in budget"],
        extractor=extract,
        runner=runner,
    )


def oscillator_jitter(
    ps: Any,
    dut: Any,
    *,
    expected_frequency: float,
    output_node: str = "out",
    output_node_b: str | None = None,
    ground: str = "0",
    cycles: int = 2000,
    settle_cycles: int = 50,
    threshold: float | None = None,
    hysteresis: float | None = None,
    offsets: tuple[float, ...] = (1e6,),
    accumulation_max: int = 16,
    points_per_period: int = 200,
) -> DesignBench:
    """Period, cycle-to-cycle and accumulated jitter, and the white-FM phase noise they imply.

    ngspice puts no device noise into a transient: only explicit ``trnoise`` sources
    move an edge. Without one in the deck the jitter is exactly 0 s and passes every
    spec, so this bench refuses to build. ``c_s`` is Demir's c = sigma_T**2 / T
    (sigma(m)**2 = c*m*T for white FM); ``phase_noise_dbc_hz_at_*`` is
    10*log10(f0**2 * c / df**2), valid only where ``jitter_growth_exponent`` (log-log
    slope of the m-cycle jitter over m = 1..``accumulation_max``) reads 0.5. Near 1,
    the noise is flicker or correlated and the L(f) figure is wrong. Edges use
    ``hysteresis`` (default 5 % of the swing).
    """
    tb = ps.Testbench(dut)
    if "trnoise" not in tb.netlist("ngspice").lower():
        raise ValueError("no trnoise source in the deck: a noiseless transient has 0 s jitter; "
                         "inject noise (trnoise) or use a simulator with transient device noise")
    period = 1.0 / expected_frequency
    tb.save(f"V({output_node})", *([f"V({output_node_b})"] if output_node_b else []))
    runner = _tran(tb, step_time=period / (2 * points_per_period), end_time=(settle_cycles + cycles) * period,
                   max_time=period / points_per_period)

    def extract(tran: Any) -> dict[str, float]:
        t, v = _signal(tran, output_node, output_node_b)
        t0 = settle_cycles * period
        w = t >= t0
        thr = 0.5 * (v[w].max() + v[w].min()) if threshold is None else threshold
        hyst = 0.05 * np.ptp(v[w]) if hysteresis is None else hysteresis
        e = _edges(t, v, thr, hysteresis=hyst)
        e = e[e >= t0]
        if len(e) < 4 * accumulation_max:
            raise ValueError(f"{len(e)} edges after settling; need >= {4 * accumulation_max}")
        p = np.diff(e)
        mean_p, sigma = float(p.mean()), float(p.std(ddof=1))
        c = sigma ** 2 / mean_p
        m = 2 ** np.arange(int(math.log2(accumulation_max)) + 1)
        acc = [np.std(e[k:] - e[:-k], ddof=1) for k in m]
        out = {
            "frequency_hz": 1.0 / mean_p,
            "period_jitter_rms_s": sigma,
            "period_jitter_pp_s": float(np.ptp(p)),
            "cycle_to_cycle_jitter_rms_s": float(np.diff(p).std(ddof=1)),
            "c_s": c,
            "jitter_growth_exponent": float(np.polyfit(np.log(m), np.log(acc), 1)[0]),
        }
        for df in offsets:
            out[spot("phase_noise_dbc_hz_at", df)] = 10 * math.log10(c / (mean_p ** 2 * df ** 2))
        return out

    return DesignBench(
        name="oscillator_jitter",
        category="oscillator",
        testbench=tb,
        intent="Period, cycle-to-cycle and accumulated jitter from injected transient noise.",
        measurements=["frequency_hz", "period_jitter_rms_s", "period_jitter_pp_s", "cycle_to_cycle_jitter_rms_s",
                      "c_s", "jitter_growth_exponent"] + [spot("phase_noise_dbc_hz_at", df) for df in offsets],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["period jitter in budget", "growth exponent near 0.5 before trusting L(f)"],
        extractor=extract,
        runner=runner,
    )


# ── clocks and delay lines ──

def clock_buffer_timing(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "in",
    output_node: str = "out",
    supply_node: str = "vdd",
    ground: str = "0",
    vdd: float = 1.8,
    frequency: float = 100e6,
    input_slew: float | None = None,
    load_capacitance: float | None = None,
    inverting: bool = False,
    cycles: int = 50,
    ripple_v: float = 0.0,
    ripple_frequency: float | None = None,
    points_per_period: int = 1000,
) -> DesignBench:
    """Propagation delay per edge, duty-cycle distortion and power-supply-induced jitter.

    A 50 %-duty clock (10-90 % ``input_slew``, default T/50) drives ``input_node``; the
    input threshold is ``vdd / 2``, the output threshold half the *instantaneous*
    supply, so supply ripple does not read as fake delay. ``dcd_s = tphl - tplh`` is
    the output high time minus the ideal one. With ``ripple_v > 0`` a sine at
    ``ripple_frequency`` (default f/7.3, so edges sample every ripple phase) rides on
    the supply: ``psij_pp_s`` is the spread of the rising-output delay and
    ``psij_s_per_v`` its least-squares amplitude at the ripple frequency per volt.
    NLDM delay/slew tables are ``characterize.delay_table``.
    """
    period = 1.0 / frequency
    edge = (input_slew if input_slew is not None else period / 50) / 0.8
    f_rip = ripple_frequency or frequency / 7.3
    tb = ps.Testbench(dut)
    if ripple_v > 0:
        tb.SinusoidalVoltageSource(name="dd", positive=supply_node, negative=ground, dc_offset=vdd,
                                   offset=vdd, amplitude=ripple_v, frequency=f_rip)
    else:
        tb.V(name="dd", positive=supply_node, negative=ground, value=vdd)
    # The first rising edge at T/4, so the DUT starts from its operating point.
    _clock(tb, "clk", input_node, ground, period / 4 + period * np.arange(cycles), period / 2, edge, vdd)
    if load_capacitance is not None:
        tb.C(name="load", positive=output_node, negative=ground, value=load_capacitance)
    tb.save(f"V({input_node})", f"V({output_node})", f"V({supply_node})")
    runner = _tran(tb, step_time=period / (2 * points_per_period), end_time=(cycles + 0.5) * period,
                   max_time=period / points_per_period)

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        vin = np.asarray(tran[input_node], float)
        y = np.asarray(tran[output_node], float) - 0.5 * np.asarray(tran[supply_node], float)
        in_r, in_f = _edges(t, vin, vdd / 2), _edges(t, vin, vdd / 2, rising=False)

        def delays(out: np.ndarray, cause: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
            k = np.searchsorted(cause, out) - 1
            return out[k >= 0], out[k >= 0] - cause[k[k >= 0]]

        t_r, d_r = delays(_edges(t, y, 0.0), in_f if inverting else in_r)
        t_f, d_f = delays(_edges(t, y, 0.0, rising=False), in_r if inverting else in_f)
        if len(d_r) < 2 or len(d_f) < 2:
            raise ValueError(f"output does not toggle: {len(d_r)} rising, {len(d_f)} falling delayed edges")
        high = t_f[np.searchsorted(t_f, t_r[:-1])] - t_r[:-1]
        m = {
            "tplh_s": float(d_r.mean()),
            "tphl_s": float(d_f.mean()),
            "dcd_s": float(d_f.mean() - d_r.mean()),
            "duty_out": float(high.mean() / period),
        }
        if ripple_v > 0:
            m["psij_pp_s"] = float(np.ptp(d_r))
            m["psij_s_per_v"] = _tone(t_r, d_r, f_rip) / ripple_v
        return m

    return DesignBench(
        name="clock_buffer_timing",
        category="clock",
        testbench=tb,
        intent="Per-edge delay, duty-cycle distortion and supply-induced jitter of a clock buffer.",
        measurements=["tplh_s", "tphl_s", "dcd_s", "duty_out"] + (["psij_pp_s", "psij_s_per_v"] if ripple_v > 0 else []),
        result_fields=["tran.time", f"tran['{input_node}']", f"tran['{output_node}']"],
        validation=["delay in budget", "duty-cycle distortion in budget", "PSIJ in budget"],
        extractor=extract,
        runner=runner,
    )


def delay_line_range(
    ps: Any,
    dut: Any,
    *,
    values: list[float],
    frequency: float,
    input_node: str = "in",
    output_node: str = "out",
    control_node: str = "vctrl",
    ground: str = "0",
    v_high: float = 1.0,
    settle_cycles: int = 20,
    measure_cycles: int = 20,
    points_per_period: int = 1000,
) -> DesignBench:
    """Delay of a voltage-controlled delay line against its control: range, Kd, lock range.

    A clock at ``frequency`` (first edge at t=0) drives the line while ``control_node``
    steps through ``values``. Output rising edge k is paired with input rising edge k
    (every edge passes; the line may be longer than a period), skipping output edges
    before the first input edge. A line that emits an extra edge when its control
    steps breaks that pairing (and would break a DLL too). ``covers_period`` (delay_min < T < delay_max) is the
    DLL lock-range condition; ``harmonic_lock_risk`` flags delay_max > 2T, where a DLL
    can lock on the wrong edge.
    """
    period = 1.0 / frequency
    hold = (settle_cycles + measure_cycles) * period
    n_clk = round(len(values) * hold / period)
    tb = ps.Testbench(dut)
    _clock(tb, "clk", input_node, ground, period * np.arange(n_clk), period / 2, period / 100, v_high)
    _step_pwl(tb, "ctl", control_node, ground, values, hold)
    tb.save(f"V({input_node})", f"V({output_node})")
    runner = _tran(tb, step_time=period / (2 * points_per_period), end_time=len(values) * hold,
                   max_time=period / points_per_period)

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        ins = _edges(t, np.asarray(tran[input_node], float), v_high / 2)
        outs = _edges(t, np.asarray(tran[output_node], float), v_high / 2)
        outs = outs[outs > ins[0]]
        n = min(len(ins), len(outs))
        d = outs[:n] - ins[:n]
        if n == 0 or np.any(d <= 0):
            raise ValueError("output edges do not follow input edges one for one (a swallowed edge?)")
        # An edge counts for a step only if its output also lands before the control
        # steps again: a line longer than T has edges in flight across the boundary.
        delay = np.array([d[(ins[:n] >= (k + 1) * hold - measure_cycles * period) & (outs[:n] < (k + 1) * hold)].mean()
                          for k in range(len(values))])
        if np.any(np.isnan(delay)):
            raise ValueError(f"no delayed edge in the measure window of step(s) {np.nonzero(np.isnan(delay))[0].tolist()}")
        mid, _ = _slopes(values, delay)
        m = {f"delay_s_{k}": float(x) for k, x in enumerate(delay)}
        m.update({
            "delay_min_s": float(delay.min()),
            "delay_max_s": float(delay.max()),
            "kd_s_per_v": mid,
            "covers_period": float(delay.min() < period < delay.max()),
            "harmonic_lock_risk": float(delay.max() > 2 * period),
        })
        return m

    return DesignBench(
        name="delay_line_range",
        category="dll",
        testbench=tb,
        intent="Controlled delay range, Kd, and whether the range covers one period.",
        measurements=[f"delay_s_{k}" for k in range(len(values))] + [
            "delay_min_s", "delay_max_s", "kd_s_per_v", "covers_period", "harmonic_lock_risk"],
        result_fields=["tran.time", f"tran['{input_node}']", f"tran['{output_node}']"],
        validation=["range covers the period", "no harmonic-lock risk", "Kd in budget"],
        extractor=extract,
        runner=runner,
    )


def crystal_negative_resistance(
    ps: Any,
    dut: Any,
    *,
    frequency: float,
    xtal_a: str = "xa",
    xtal_b: str = "xb",
    motional_resistance: float | None = None,
    span: float = 0.1,
    points: int = 201,
) -> DesignBench:
    """Small-signal startup margin of a crystal-oscillator core, the crystal removed.

    A 1 A AC current (0 A DC, so the bias is untouched) flows into ``xtal_a`` and out of
    ``xtal_b``; Zc = V(a) - V(b) is the impedance the core presents to the crystal,
    swept linearly over ``frequency * (1 +- span)``. ``negative_resistance_ohm`` is
    -Re(Zc) at ``frequency`` (positive: oscillation possible), ``load_capacitance_f``
    -1/(w Im Zc), the load the crystal sees. Include C0 and board parasitics in the
    DUT, or the margin is optimistic. Drive level needs a transient or HB.
    """
    tb = ps.Testbench(dut)
    tb.I(name="xtal", positive=xtal_b, negative=xtal_a, value=0.0, ac=1.0)
    tb.save(f"V({xtal_a})", f"V({xtal_b})")
    ac_args = dict(variation="lin", number_of_points=points,
                   start_frequency=frequency * (1 - span), stop_frequency=frequency * (1 + span))
    tb.add_ac(**ac_args)

    def extract(ac: Any) -> dict[str, float]:
        f = np.asarray(ac.frequency, float)
        z = ac_complex(ac, xtal_a) - ac_complex(ac, xtal_b)
        re, im = float(np.interp(frequency, f, z.real)), float(np.interp(frequency, f, z.imag))
        m = {"negative_resistance_ohm": -re, "load_capacitance_f": -1.0 / (2 * math.pi * frequency * im)}
        if motional_resistance is not None:
            m["negative_resistance_margin"] = -re / motional_resistance
        return m

    return DesignBench(
        name="crystal_negative_resistance",
        category="oscillator",
        testbench=tb,
        intent="Negative resistance and load capacitance a crystal-oscillator core presents to the crystal.",
        measurements=["negative_resistance_ohm", "load_capacitance_f"]
        + (["negative_resistance_margin"] if motional_resistance is not None else []),
        result_fields=["ac.frequency", f"ac['{xtal_a}']", f"ac['{xtal_b}']"],
        validation=["negative resistance at least 3-5x the motional resistance"],
        extractor=extract,
        runner=lambda: tb.ac(**ac_args),
    )
