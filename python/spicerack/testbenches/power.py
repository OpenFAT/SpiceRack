"""Regulator, reference and supply benches: PSRR, line and load regulation,
dropout, load and line steps, start-up from off, loop margins, average power.

Every bench wraps a ``Subcircuit`` DUT in V/I/R/C stimulus and owns its runs:
``bench.measure()`` returns the metrics. Currents keep SPICE's sign, where
``I(Vx)`` flows into the ``+`` node of Vx, so a supply delivers ``-I(Vdd)``.
``bias`` maps extra DUT pins (references, enables) to DC volts; those
sources carry no AC, so they never add to a PSRR or loop-gain response.
"""

from __future__ import annotations

import math
from typing import Any, Mapping, Sequence

import numpy as np

from ..remeasure import _margins, _settling
from ._common import spot, tian_loop_gain
from .analysis import _interp_at, crossings
from .characterize import _integral
from .design import DesignBench


def _hold(tb: Any, bias: Mapping[str, float] | None, ground: str) -> None:
    for pin, volts in (bias or {}).items():
        tb.V(name=f"bias_{pin}", positive=pin, negative=ground, value=float(volts))


def _load(tb: Any, node: str, ground: str, current: float | None, resistance: float | None) -> None:
    """Resistor, or an ideal sink of ``current`` out of ``node``."""
    if resistance is not None:
        tb.R(name="load", positive=node, negative=ground, value=resistance)
    elif current:
        tb.I(name="load", positive=node, negative=ground, value=current)


def _output_cap(tb: Any, node: str, ground: str, c: float | None, esr: float) -> None:
    """An off-chip C_out with its series ESR: part of the harness, not the DUT."""
    if c is None:
        return
    if esr:
        tb.R(name="esr", positive=node, negative="cout_esr", value=esr)
        node = "cout_esr"
    tb.C(name="out", positive=node, negative=ground, value=c)


def average_power(tran: Any, supply_source: str, t0: float, t1: float, *, node: str) -> float:
    """Mean power the V source ``supply_source`` (element name, e.g. ``"Vdd"``)
    delivers over ``[t0, t1]``: ``(1/(t1-t0)) * integral v(node) * -i dt``.

    Trapezoid over the simulator's own time points, the window ends
    interpolated. Pick an integer number of periods of a settled waveform:
    a switched C then reads ``C * V**2 * f``.
    """
    if not t1 > t0:
        raise ValueError(f"need t1 > t0 (t0={t0}, t1={t1})")
    t = np.asarray(tran.time, float)
    p = np.asarray(tran[node], float) * -np.asarray(tran[f"i({supply_source.lower()})"], float)
    return _integral(t, p, t0, t1) / (t1 - t0)


def supply_rejection(
    ps: Any,
    dut: Any,
    *,
    supply_node: str = "vdd",
    output_node: str = "vout",
    ground: str = "0",
    supply_v: float = 1.8,
    bias: Mapping[str, float] | None = None,
    load_current: float | None = None,
    load_resistance: float | None = None,
    output_capacitance: float | None = None,
    esr: float = 0.0,
    start_frequency: float = 10.0,
    stop_frequency: float = 10e6,
    points_per_decade: int = 20,
    spot_hz: Sequence[float] = (1e3, 1e5, 1e6),
) -> DesignBench:
    """PSRR vs frequency of a closed-loop block (regulator, reference, op amp
    in its application circuit), output-referred and positive for rejection:
    ``-20 log10 |v_out / v_dd|``. The supply carries the only ``ac=1``.

    An op amp's datasheet PSRR is input-referred: divide by its closed-loop
    gain. The load and C_out/ESR are part of the spec: PSRR at no load, or
    with a resistive instead of a current load, is a different number.
    """
    bad = [f for f in spot_hz if not start_frequency <= f <= stop_frequency]
    if bad:
        raise ValueError(f"spot_hz {bad} outside the sweep [{start_frequency}, {stop_frequency}] Hz")
    tb = ps.Testbench(dut)
    tb.V(name="dd", positive=supply_node, negative=ground, value=supply_v, ac=1.0)
    _hold(tb, bias, ground)
    _load(tb, output_node, ground, load_current, load_resistance)
    _output_cap(tb, output_node, ground, output_capacitance, esr)
    ac_args = dict(variation="dec", number_of_points=points_per_decade,
                   start_frequency=start_frequency, stop_frequency=stop_frequency)
    tb.add_operating_point()
    tb.add_ac(**ac_args)
    spots = {spot("psrr_db_at", f): f for f in spot_hz}

    def extract(ac: Any) -> dict[str, float]:
        f = np.asarray(ac.frequency, float)
        psrr = -np.asarray(ac.magnitude_db(output_node), float)
        worst = int(np.argmin(psrr))
        out = {"psrr_dc_db": float(psrr[0]), "psrr_min_db": float(psrr[worst]), "psrr_min_hz": float(f[worst])}
        out.update({name: _interp_at(np.log10(f), psrr, math.log10(fs)) for name, fs in spots.items()})
        return out

    return DesignBench(
        name="supply_rejection",
        category="power",
        testbench=tb,
        intent="Output-referred supply rejection vs frequency at the regulated, loaded bias point.",
        measurements=["psrr_dc_db", "psrr_min_db", "psrr_min_hz", *spots],
        result_fields=["ac.frequency", f"ac.magnitude_db('{output_node}')"],
        validation=["PSRR at DC and at each spot above target", "worst-case PSRR above target"],
        extractor=extract,
        runner=lambda: tb.ac(**ac_args),
    )


def regulator_line_sweep(
    ps: Any,
    dut: Any,
    *,
    vout_nominal: float,
    vin_nominal: float,
    vin_max: float,
    input_node: str = "vin",
    output_node: str = "vout",
    ground: str = "0",
    vin_start: float = 0.0,
    vin_step: float = 5e-3,
    vin_reg_min: float | None = None,
    dropout_criterion: float = 0.1,
    load_current: float = 10e-3,
    load_resistance: float | None = None,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """Line regulation, dropout and ground current from one DC sweep of V_in at
    a fixed load.

    The slope window ``[vin_reg_min, vin_max]`` (default from ``vin_nominal``)
    is separate from the dropout search so the knee never bends the slope.
    ``dropout_v`` is ``V_in - V_out`` at the highest V_in where V_out sits
    ``dropout_criterion`` volts below ``vout_nominal`` (the 100 mV convention by
    default; datasheets also use 1-3 %). Run it at the maximum load.
    ``ground_current_a`` is the input current minus the load current.
    """
    tb = ps.Testbench(dut)
    tb.V(name="in", positive=input_node, negative=ground, value=vin_nominal)
    _hold(tb, bias, ground)
    _load(tb, output_node, ground, load_current, load_resistance)
    sweep = dict(Vin=slice(vin_start, vin_max, vin_step))
    tb.add_dc(**sweep)
    reg_lo = vin_nominal if vin_reg_min is None else vin_reg_min
    threshold = vout_nominal - dropout_criterion

    def extract(dc: Any) -> dict[str, float]:
        vin = np.asarray(dc.sweep, float)
        vout = np.asarray(dc[output_node], float)
        i_out = vout / load_resistance if load_resistance is not None else load_current or 0.0
        i_gnd = -np.asarray(dc["i(vin)"], float) - i_out
        rises = crossings(vin, vout, threshold, rising=True)
        if not rises:
            raise ValueError(f"V_out never rises through {threshold} V: start the sweep below dropout "
                             f"(it starts at {vin[0]} V)")
        v_nom = _interp_at(vin, vout, vin_nominal)
        slope = (vout[-1] - _interp_at(vin, vout, reg_lo)) / (vin[-1] - reg_lo)
        return {
            "vout_v": v_nom,
            "line_regulation_v_per_v": float(slope),
            "line_regulation_pct_per_v": float(100.0 * slope / v_nom),
            "dropout_v": rises[-1] - threshold,
            "ground_current_a": _interp_at(vin, i_gnd, vin_nominal),
        }

    return DesignBench(
        name="regulator_line_sweep",
        category="power",
        testbench=tb,
        intent="Line regulation, dropout (named criterion) and ground current vs V_in at a fixed load.",
        measurements=["vout_v", "line_regulation_v_per_v", "line_regulation_pct_per_v",
                      "dropout_v", "ground_current_a"],
        result_fields=["dc.sweep", f"dc['{output_node}']", "dc['i(vin)']"],
        validation=["line regulation below target", "dropout below target at max load",
                    "ground current in budget"],
        extractor=extract,
        runner=lambda: tb.dc(**sweep),
    )


def regulator_load_sweep(
    ps: Any,
    dut: Any,
    *,
    vin_nominal: float,
    input_node: str = "vin",
    output_node: str = "vout",
    ground: str = "0",
    i_load_min: float = 0.0,
    i_load_max: float = 10e-3,
    i_load_step: float | None = None,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """Load regulation and quiescent current from a DC sweep of an ideal load.

    ``load_regulation_ohm`` and ``load_regulation_pct`` are positive for a
    droop. An ideal current load past the current limit has no solution, so
    keep ``i_load_max`` inside the limit; the limit itself is a resistive-load
    sweep, not this bench.
    """
    tb = ps.Testbench(dut)
    tb.V(name="in", positive=input_node, negative=ground, value=vin_nominal)
    tb.I(name="load", positive=output_node, negative=ground, value=i_load_min)
    _hold(tb, bias, ground)
    sweep = dict(Iload=slice(i_load_min, i_load_max, i_load_step or (i_load_max - i_load_min) / 100.0))
    tb.add_dc(**sweep)

    def extract(dc: Any) -> dict[str, float]:
        i_load = np.asarray(dc.sweep, float)
        vout = np.asarray(dc[output_node], float)
        i_gnd = -np.asarray(dc["i(vin)"], float) - i_load
        drop = float(vout[0] - vout[-1])
        return {
            "load_regulation_ohm": drop / float(i_load[-1] - i_load[0]),
            "load_regulation_pct": 100.0 * drop / float(vout[0]),
            "quiescent_current_a": float(i_gnd[0]),
            "ground_current_max_load_a": float(i_gnd[-1]),
        }

    return DesignBench(
        name="regulator_load_sweep",
        category="power",
        testbench=tb,
        intent="Load regulation, quiescent current and ground current vs load current at fixed V_in.",
        measurements=["load_regulation_ohm", "load_regulation_pct", "quiescent_current_a",
                      "ground_current_max_load_a"],
        result_fields=["dc.sweep", f"dc['{output_node}']", "dc['i(vin)']"],
        validation=["load regulation below target", "quiescent current in budget"],
        extractor=extract,
        runner=lambda: tb.dc(**sweep),
    )


def regulator_step(
    ps: Any,
    dut: Any,
    *,
    kind: str = "load",
    vin_nominal: float,
    input_node: str = "vin",
    output_node: str = "vout",
    ground: str = "0",
    i_low: float = 1e-3,
    i_high: float = 10e-3,
    vin_low: float | None = None,
    vin_high: float | None = None,
    t_edge: float | None = None,
    delay: float | None = None,
    hold: float | None = None,
    output_capacitance: float | None = None,
    esr: float = 0.0,
    band_v: float | None = None,
    max_time: float | None = None,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """Load step (``i_low -> i_high -> i_low``) or line step (``vin_low ->
    vin_high -> vin_low``, ``vin_low`` defaulting to ``vin_nominal``, at a
    constant ``i_low`` load), each edge ``t_edge``
    long (1 us load, 10 us line by default), first edge at ``delay``, held
    ``hold``. The run starts at the DC point of the low level.

    ``undershoot_v`` / ``overshoot_v``: the dip and the rise past the pre-edge
    level after the edge that pulls V_out down / up. ``settling_up_s`` /
    ``settling_down_s``: from the start of the up / down edge of the stimulus
    until V_out stays within ``band_v`` (default 1 % of V_out) of its post-edge
    final value, which load or line regulation has shifted. ``final_shift_v``:
    final V_out after the up edge minus before it. The edge rate is part of
    the spec: undershoot scales with dI/dt for slow edges.
    """
    if kind not in ("load", "line"):
        raise ValueError(f"kind must be 'load' or 'line', not {kind!r}")
    if kind == "line" and vin_high is None:
        raise ValueError("a line step needs vin_high")
    t_edge = t_edge if t_edge is not None else (1e-6 if kind == "load" else 10e-6)
    hold = hold if hold is not None else 50.0 * t_edge
    t1 = delay if delay is not None else 5.0 * t_edge
    t2, end = t1 + hold, t1 + 2.0 * hold
    max_time = max_time if max_time is not None else t_edge / 20.0

    def wave(lo: float, hi: float) -> list[tuple[float, float]]:
        return [(0.0, lo), (t1, lo), (t1 + t_edge, hi), (t2, hi), (t2 + t_edge, lo), (end, lo)]

    tb = ps.Testbench(dut)
    _hold(tb, bias, ground)
    if kind == "load":
        tb.V(name="in", positive=input_node, negative=ground, value=vin_nominal)
        tb.PieceWiseLinearCurrentSource(name="load", positive=output_node, negative=ground,
                                        values=wave(i_low, i_high))
    else:
        lo = vin_nominal if vin_low is None else vin_low
        tb.PieceWiseLinearVoltageSource(name="in", positive=input_node, negative=ground, values=wave(lo, vin_high))
        _load(tb, output_node, ground, i_low, None)
    _output_cap(tb, output_node, ground, output_capacitance, esr)
    run = dict(step_time=max_time, end_time=end, max_time=max_time)
    tb.add_transient(**run)

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        y = np.asarray(tran[output_node], float)

        def edge(ta: float, tb_: float, dips: bool) -> tuple[float, float, float]:
            w = (t >= ta) & (t <= tb_)
            v0, final = _interp_at(t, y, ta), _interp_at(t, y, tb_)
            peak = v0 - float(y[w].min()) if dips else float(y[w].max()) - v0
            band = band_v if band_v is not None else 0.01 * abs(v0)
            return peak, _settling(t[w], y[w], final, t0=ta, abs_tol=band), final - v0

        up = edge(t1, t2, dips=kind == "load")
        down = edge(t2, end, dips=kind == "line")
        dip, rise = (up, down) if kind == "load" else (down, up)
        return {
            "undershoot_v": dip[0],
            "overshoot_v": rise[0],
            "settling_up_s": up[1],
            "settling_down_s": down[1],
            "final_shift_v": up[2],
        }

    return DesignBench(
        name="regulator_step",
        category="power",
        testbench=tb,
        intent=f"{kind.capitalize()}-step transient: undershoot, overshoot and recovery to the shifted final value.",
        measurements=["undershoot_v", "overshoot_v", "settling_up_s", "settling_down_s", "final_shift_v"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["undershoot and overshoot in budget", "recovery within the settling budget"],
        extractor=extract,
        runner=lambda: tb.transient(**run),
    )


def supply_startup(
    ps: Any,
    dut: Any,
    *,
    v_expected: float,
    supply_node: str = "vdd",
    output_node: str = "vout",
    ground: str = "0",
    vdd: float = 1.8,
    band: float = 0.01,
    ramp_time: float = 10e-6,
    settle_window: float | None = None,
    max_time: float | None = None,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """Start-up from a genuinely off state: the supply ramps 0 -> ``vdd`` over
    ``ramp_time`` and holds for ``settle_window`` (default 10 ramps). No
    ``uic``, no ``.ic``: the t=0 operating point with the supply at 0 is the
    off state.

    ``v_expected`` comes from the spec, never from a DC point at full supply:
    a self-biased loop's DC point can be its zero-current state. ``started``
    is 1 when V_out ends within ``band * |v_expected|``. ``startup_time_s`` is
    from t=0 until V_out enters that band for good (inf when it never does).
    Run at least a fast and a slow ramp: slow ramps defeat start-up circuits
    that rely on dV/dt coupling.
    """
    settle_window = settle_window if settle_window is not None else 10.0 * ramp_time
    end = ramp_time + settle_window
    tb = ps.Testbench(dut)
    tb.PieceWiseLinearVoltageSource(name="dd", positive=supply_node, negative=ground,
                                    values=[(0.0, 0.0), (ramp_time, vdd), (end, vdd)])
    _hold(tb, bias, ground)
    run = dict(step_time=ramp_time / 1000.0, end_time=end,
               max_time=max_time if max_time is not None else ramp_time / 100.0)
    tb.add_transient(**run)
    tol_v = band * abs(v_expected)

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        y = np.asarray(tran[output_node], float)
        t_s = _settling(t, y, v_expected, abs_tol=tol_v)
        return {
            "started": float(abs(y[-1] - v_expected) <= tol_v),
            "startup_time_s": t_s,
            "startup_after_supply_s": t_s - ramp_time,
            "startup_overshoot_v": max(0.0, float(y.max()) - v_expected),
            "inrush_peak_a": float(np.max(-np.asarray(tran["i(vdd)"], float))),
        }

    return DesignBench(
        name="supply_startup",
        category="power",
        testbench=tb,
        intent="Start-up from an off state under a supply ramp: whether, when, overshoot and inrush.",
        measurements=["started", "startup_time_s", "startup_after_supply_s", "startup_overshoot_v",
                      "inrush_peak_a"],
        result_fields=["tran.time", f"tran['{output_node}']", "tran['i(vdd)']"],
        validation=["started at every corner and ramp", "start-up time in budget", "inrush in budget"],
        extractor=extract,
        runner=lambda: tb.transient(**run),
    )


def regulator_loop_margins(
    ps: Any,
    dut: Any,
    *,
    vin_nominal: float,
    vout_nominal: float,
    input_node: str = "vin",
    output_node: str = "vout",
    ground: str = "0",
    feedback_node: str = "fb",
    loop_output: str | None = None,
    divider: tuple[float, float] | None = None,
    load_currents: Sequence[float] = (0.0,),
    cout_esr: Sequence[tuple[float, float]] = ((1e-6, 0.0),),
    start_frequency: float = 1.0,
    stop_frequency: float = 1e9,
    points_per_decade: int = 50,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """Phase and gain margin of the regulation loop over a grid of resistive
    loads (``R_L = vout_nominal / I_L``; 0 A means no load) and C_out/ESR pairs,
    by Tian's double injection at a loop break (exact whatever the impedance
    there; ``stb`` is Spectre-only).

    The break sits between ``feedback_node`` (the loop's input, e.g. the FB
    pin) and ``loop_output``. With ``divider=(r_top, r_bottom)`` the bench
    builds the divider from ``output_node`` and the break is between its tap
    and ``feedback_node``. A grid point whose loop gain never crosses 1 in the
    sweep raises; ``gain_margin_db_min`` is inf when no point's phase reaches
    -180 deg inside the sweep.
    """
    if (divider is None) == (loop_output is None):
        raise ValueError("name exactly one of loop_output (a split pair) and divider (an FB pin)")
    ac_args = dict(variation="dec", number_of_points=points_per_decade,
                   start_frequency=start_frequency, stop_frequency=stop_frequency)
    grid = [(i_l, c, esr) for i_l in load_currents for c, esr in cout_esr]

    def build(point: tuple[float, float, float], v_ac: float, i_ac: float) -> Any:
        i_l, c, esr = point
        tb = ps.Testbench(dut)
        tb.V(name="in", positive=input_node, negative=ground, value=vin_nominal)
        _hold(tb, bias, ground)
        tap = loop_output
        if divider is not None:
            tap = "fb_tap"
            tb.R(name="fb_top", positive=output_node, negative=tap, value=divider[0])
            tb.R(name="fb_bot", positive=tap, negative=ground, value=divider[1])
        _load(tb, output_node, ground, None, vout_nominal / i_l if i_l else None)
        _output_cap(tb, output_node, ground, c, esr)
        tb.V(name="prb", positive=feedback_node, negative=tap, value=0.0, ac=v_ac)
        tb.I(name="prb", positive=ground, negative=feedback_node, value=0.0, ac=i_ac)
        return tb

    def run() -> dict[tuple[float, float, float], tuple[np.ndarray, np.ndarray]]:
        return {p: tian_loop_gain(lambda v, i, p=p: build(p, v, i), ac_args, drive=feedback_node) for p in grid}

    def extract(loops: Mapping[tuple[float, float, float], tuple[np.ndarray, np.ndarray]]) -> dict[str, float]:
        rows = {p: _margins(f, w) for p, (f, w) in loops.items()}
        lost = [p for p, m in rows.items() if math.isnan(m["pm_deg"])]
        if lost:
            raise ValueError(f"loop gain never crosses 1 inside the sweep at (I_L, C, ESR) = {lost}")
        worst = min(rows, key=lambda p: rows[p]["pm_deg"])
        gms = [m["gm_db"] for m in rows.values() if not math.isnan(m["gm_db"])]
        return {
            "phase_margin_deg_min": rows[worst]["pm_deg"],
            "gain_margin_db_min": min(gms) if gms else math.inf,
            "unity_gain_hz_at_worst": rows[worst]["ugf_hz"],
            "worst_load_a": worst[0],
            "worst_cout_f": worst[1],
            "worst_esr_ohm": worst[2],
        }

    return DesignBench(
        name="regulator_loop_margins",
        category="power",
        testbench=build(grid[0], 1.0, 0.0),
        intent="Regulation-loop phase and gain margin over load and C_out/ESR, by Tian injection.",
        measurements=["phase_margin_deg_min", "gain_margin_db_min", "unity_gain_hz_at_worst",
                      "worst_load_a", "worst_cout_f", "worst_esr_ohm"],
        result_fields=["ac.frequency", "i(vprb)", f"ac['{feedback_node}']"],
        validation=["phase margin above target at every grid point", "gain margin above target"],
        extractor=extract,
        runner=run,
    )
