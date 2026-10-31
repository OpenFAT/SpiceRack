"""Current-mode and charge-based benches: mirror output, current reference,
transconductor G_m, switched-capacitor transfer, charge-pump currents.

Each bench drives a ``Subcircuit`` DUT, owns its runs (``bench.measure()``)
and reports currents with the sign stated in its docstring. ``bias`` maps
extra DUT pins (supplies, cascode gates) to DC volts.
"""

from __future__ import annotations

import json
import math
from typing import Any, Mapping

import numpy as np

from ..remeasure import _settling
from .analysis import _interp_at, crossings
from .design import DesignBench, _sample
from .power import _hold


def _nearest(hits: list[float], center: float) -> tuple[float, float]:
    """The crossings nearest ``center`` below and above it (nan for none)."""
    below = [x for x in hits if x < center]
    above = [x for x in hits if x > center]
    return (max(below) if below else math.nan, min(above) if above else math.nan)


def current_mirror_output(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "iin",
    output_node: str = "iout",
    ground: str = "0",
    input_current: float = 10e-6,
    nominal_ratio: float = 1.0,
    vout_nominal: float = 0.9,
    vout_start: float = 0.0,
    vout_stop: float = 1.8,
    vout_step: float | None = None,
    compliance_tolerance: float = 0.01,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """Ratio, output resistance and compliance from one DC sweep of the output
    voltage. Currents are positive *into* the DUT pin: ``input_current`` > 0
    feeds an NMOS (sink) mirror, < 0 draws from a PMOS (source) one.

    ``rout_ohm`` is 1/(dI_out/dV_out) at ``vout_nominal`` by central
    difference; the sweep runs at reltol 1e-6 so the derivative is not
    solver noise. ``compliance_v`` is where |I_out| sags by
    ``compliance_tolerance`` below its nominal value, searched below nominal
    for a sink and above for a source (from the current's sign).
    ``compliance_rout_v`` is where r_out falls to a tenth of nominal, which
    happens before the current visibly sags. nan: no such point in the sweep.
    """
    tb = ps.Testbench(dut)
    tb.I(name="in", positive=ground, negative=input_node, value=input_current)
    tb.V(name="out", positive=output_node, negative=ground, value=vout_nominal)
    _hold(tb, bias, ground)
    tb.options(reltol=1e-6)
    sweep = dict(Vout=slice(vout_start, vout_stop, vout_step or (vout_stop - vout_start) / 200.0))
    tb.add_dc(**sweep)

    def extract(dc: Any) -> dict[str, float]:
        v = np.asarray(dc.sweep, float)
        i = -np.asarray(dc["i(vout)"], float)
        g = np.gradient(i, v)
        i_nom, g_nom = _interp_at(v, i, vout_nominal), _interp_at(v, g, vout_nominal)
        sink = i_nom > 0
        sag = _nearest(crossings(v, abs(i), (1 - compliance_tolerance) * abs(i_nom)), vout_nominal)
        knee = _nearest(crossings(v, g, 10.0 * g_nom), vout_nominal)
        return {
            "iout_a": i_nom,
            "ratio_error": i_nom / (nominal_ratio * input_current) - 1.0,
            "rout_ohm": 1.0 / g_nom,
            "compliance_v": sag[0] if sink else sag[1],
            "compliance_rout_v": knee[0] if sink else knee[1],
            "vin_v": _interp_at(v, np.asarray(dc[input_node], float), vout_nominal),
        }

    return DesignBench(
        name="current_mirror_output",
        category="current_mode",
        testbench=tb,
        intent="Mirror ratio error, output resistance, current and r_out compliance from an output sweep.",
        measurements=["iout_a", "ratio_error", "rout_ohm", "compliance_v", "compliance_rout_v", "vin_v"],
        result_fields=["dc.sweep", "dc['i(vout)']", f"dc['{input_node}']"],
        validation=["ratio error in budget", "r_out above target", "compliance below the output swing"],
        extractor=extract,
        runner=lambda: tb.dc(**sweep),
    )


def current_reference(
    ps: Any,
    dut: Any,
    *,
    sweep: str = "temp",
    supply_node: str = "vdd",
    output_node: str = "iref",
    ground: str = "0",
    supply: float = 1.8,
    output_bias_v: float = 0.9,
    t_min: float = -40.0,
    t_max: float = 125.0,
    t_step: float = 5.0,
    supply_start: float | None = None,
    supply_stop: float | None = None,
    supply_step: float = 10e-3,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """Drift of an output current (PTAT, CTAT, constant-g_m, beta multiplier)
    with temperature (``.dc temp``) or supply, the output held at
    ``output_bias_v``, which the answer depends on through r_out. The current
    is positive into the DUT pin.

    ``sweep="temp"``: ``iref_27c_a``, ``tempco_box_ppm_c`` (as
    ``bandgap_tempco``) and ``ptat_intercept_c``, where the linear fit of
    I(T) crosses 0 (-273.15 for pure PTAT). ``sweep="supply"`` over
    ``[supply_start, supply_stop]`` (default ``[supply/2, supply]``): ``iref_a`` at ``supply`` and
    ``supply_sensitivity_pct_per_v`` = 100 dI / (I dV) over the window. A DC
    sweep can ride a self-biased loop's zero-current state: prove start-up
    with ``supply_startup``.
    """
    if sweep not in ("temp", "supply"):
        raise ValueError(f"sweep must be 'temp' or 'supply', not {sweep!r}")
    tb = ps.Testbench(dut)
    tb.V(name="dd", positive=supply_node, negative=ground, value=supply)
    tb.V(name="out", positive=output_node, negative=ground, value=output_bias_v)
    _hold(tb, bias, ground)
    tb.nominal_temperature = 27.0
    if sweep == "temp":
        run = dict(temp=slice(t_min, t_max, t_step))
        names = ["iref_27c_a", "tempco_box_ppm_c", "ptat_intercept_c"]
    else:
        lo = supply_start if supply_start is not None else 0.5 * supply
        run = dict(Vdd=slice(lo, supply_stop if supply_stop is not None else supply, supply_step))
        names = ["iref_a", "supply_sensitivity_pct_per_v"]
    tb.add_dc(**run)

    def extract(dc: Any) -> dict[str, float]:
        x = np.asarray(dc.sweep, float)
        i = -np.asarray(dc["i(vout)"], float)
        if sweep == "supply":
            i_ref = _interp_at(x, i, supply)
            return {"iref_a": i_ref,
                    "supply_sensitivity_pct_per_v": float(100.0 * (i[-1] - i[0]) / (i_ref * (x[-1] - x[0])))}
        i27 = _interp_at(x, i, 27.0)
        slope, offset = np.polyfit(x, i, 1)
        return {"iref_27c_a": i27,
                "tempco_box_ppm_c": float(1e6 * np.ptp(i) / (abs(i27) * (x[-1] - x[0]))),
                "ptat_intercept_c": float(-offset / slope)}

    return DesignBench(
        name="current_reference",
        category="current_mode",
        testbench=tb,
        intent=f"Reference current drift over {sweep}, output held at a stated voltage.",
        measurements=names,
        result_fields=["dc.sweep", "dc['i(vout)']"],
        validation=["current at 27 C in range", "tempco or supply sensitivity below target"],
        extractor=extract,
        runner=lambda: tb.dc(**run),
    )


def transconductor_gm(
    ps: Any,
    dut: Any,
    *,
    input_p: str = "inp",
    input_n: str = "inn",
    output_node: str = "out",
    ground: str = "0",
    vcm: float = 0.9,
    output_bias_v: float = 0.9,
    vid_max: float = 0.5,
    vid_step: float = 1e-3,
    linearity_tolerance: float = 0.01,
    balanced_current: float = 0.0,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """G_m vs differential input at a fixed common mode, the output held at
    ``output_bias_v`` (G_m is defined output-shorted). I_out is the current
    *out of* the output pin, so a non-inverting OTA has G_m > 0.

    Two E sources in a harness subcircuit make ``inp = vcm + vid/2`` and
    ``inn = vcm - vid/2`` from one swept ``Vid``. ``offset_v`` is where I_out
    crosses ``balanced_current`` (0 for a differential output), ``gm0_s`` the
    central-difference slope there, ``linear_range_v`` the half-width around
    the offset where G_m stays within ``linearity_tolerance`` of ``gm0_s``
    (nan when the sweep ends first).
    """
    spec = json.loads(dut.to_json())
    harness = ps.Subcircuit(f"{spec['name']}_gm_harness", [])
    harness.instance(dut, "dut", *(p["name"] for p in spec["ports"]))
    for name, node, gain in (("idp", input_p, 0.5), ("idn", input_n, -0.5)):
        harness.E(name=name, positive=node, negative="gm_cm", control_positive="gm_vid",
                  control_negative=ground, voltage_gain=gain)
    tb = ps.Testbench(harness)
    tb.add_subcircuit(dut)
    tb.V(name="cm", positive="gm_cm", negative=ground, value=vcm)
    tb.V(name="id", positive="gm_vid", negative=ground, value=0.0)
    tb.V(name="out", positive=output_node, negative=ground, value=output_bias_v)
    _hold(tb, bias, ground)
    run = dict(Vid=slice(-vid_max, vid_max, vid_step))
    tb.add_dc(**run)

    def extract(dc: Any) -> dict[str, float]:
        x = np.asarray(dc.sweep, float)
        i = np.asarray(dc["i(vout)"], float)
        zeros = crossings(x, i, balanced_current)
        if not zeros:
            raise ValueError(f"I_out never crosses {balanced_current} A over Vid = +-{vid_max} V")
        offset = min(zeros, key=abs)
        g = np.gradient(i, x)
        g0 = _interp_at(x, g, offset)
        left, right = _nearest(crossings(x, g / g0, 1.0 - linearity_tolerance), offset)
        return {
            "gm0_s": g0,
            "linear_range_v": (right - left) / 2.0,
            "iout_max_a": float(np.max(np.abs(i))),
            "offset_v": offset,
        }

    return DesignBench(
        name="transconductor_gm",
        category="current_mode",
        testbench=tb,
        intent="Transconductance vs differential input: G_m0, linear range, saturation current, offset.",
        measurements=["gm0_s", "linear_range_v", "iout_max_a", "offset_v"],
        result_fields=["dc.sweep", "dc['i(vout)']"],
        validation=["gm0 in range", "linear range above target", "offset in budget"],
        extractor=extract,
        runner=lambda: tb.dc(**run),
    )


def switched_cap_transfer(
    ps: Any,
    dut: Any,
    *,
    nominal_gain: float,
    input_node: str = "vin",
    output_node: str = "vout",
    phi1_node: str = "phi1",
    phi2_node: str = "phi2",
    ground: str = "0",
    v_in: float = 0.1,
    t_clk: float = 1e-6,
    n_cycles: int = 32,
    t_gap: float | None = None,
    logic_high: float = 1.0,
    settle_tol: float = 0.01,
    max_time: float | None = None,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """Charge gain, leak and settling of an SC amplifier or integrator.

    The bench owns two non-overlapping clocks: in cycle n, phi1 is high over
    ``[nT + gap, nT + T/2 - gap]`` and phi2 over ``[nT + T/2 + gap, (n+1)T - gap]``
    (edges T/1000). The input steps at nT through ``v_in * (-1, 1, 1, ...)`` so
    the fit stays well posed for a reset stage, where V_o[n-1] carries no
    information. V_out is sampled as phi2 starts to fall and fitted to
    ``V_o[n] = leak_factor * V_o[n-1] + charge_gain * V_in[n]``. The sign of the
    gain follows the switch phasing. ``settling_time_s``: the last phi2
    window, from its rising edge to within ``settle_tol`` of the step.
    """
    gap = t_gap if t_gap is not None else t_clk / 100.0
    edge = t_clk / 1000.0
    levels = v_in * np.where(np.arange(n_cycles) % 3 == 0, -1.0, 1.0)
    tb = ps.Testbench(dut)
    points = [(0.0, float(levels[0]))]
    for k in range(1, n_cycles):
        points += [(k * t_clk, float(levels[k - 1])), (k * t_clk + edge, float(levels[k]))]
    tb.PieceWiseLinearVoltageSource(name="in", positive=input_node, negative=ground, values=points)
    for name, node, start in (("phi1", phi1_node, gap), ("phi2", phi2_node, t_clk / 2 + gap)):
        width = t_clk / 2 - 2 * gap
        wave = [(0.0, 0.0)]
        for k in range(n_cycles):
            t0 = k * t_clk + start
            wave += [(t0, 0.0), (t0 + edge, logic_high), (t0 + width, logic_high), (t0 + width + edge, 0.0)]
        tb.PieceWiseLinearVoltageSource(name=name, positive=node, negative=ground, values=wave)
    _hold(tb, bias, ground)
    step = max_time if max_time is not None else edge
    run = dict(step_time=step, end_time=n_cycles * t_clk, max_time=step)
    tb.add_transient(**run)

    def extract(tran: Any) -> dict[str, float]:
        ends = (np.arange(n_cycles) + 1) * t_clk - gap
        vo = _sample(tran, output_node, ends)
        (leak, gain), *_ = np.linalg.lstsq(np.column_stack([vo[:-1], levels[1:]]), vo[1:], rcond=None)
        t = np.asarray(tran.time, float)
        y = np.asarray(tran[output_node], float)
        t0 = ends[-1] - t_clk / 2 + 2 * gap
        w = (t >= t0) & (t <= ends[-1])
        return {
            "charge_gain": float(gain),
            "leak_factor": float(leak),
            "gain_error": float(gain / nominal_gain - 1.0),
            "settling_time_s": _settling(t[w], y[w], float(vo[-1]), settle_tol, t0),
        }

    return DesignBench(
        name="switched_cap_transfer",
        category="charge",
        testbench=tb,
        intent="Switched-capacitor charge gain, gain error, leak and per-phase settling.",
        measurements=["charge_gain", "leak_factor", "gain_error", "settling_time_s"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["gain error in budget", "settles within phi2"],
        extractor=extract,
        runner=lambda: tb.transient(**run),
    )


def charge_pump_current(
    ps: Any,
    dut: Any,
    *,
    output_node: str = "cp",
    up_node: str = "up",
    dn_node: str = "dn",
    ground: str = "0",
    up_levels: tuple[float, float] = (0.0, 1.0),
    dn_levels: tuple[float, float] = (0.0, 1.0),
    vout_min: float = 0.0,
    vout_max: float = 1.8,
    vout_step: float = 5e-3,
    vout_mid: float | None = None,
    tolerance: float = 0.05,
    bias: Mapping[str, float] | None = None,
) -> DesignBench:
    """UP and DN currents vs output voltage from three DC sweeps of the output
    (UP only, DN only, both on), each switch at its ``(off, on)`` level.

    ``i_up_a`` (sourced out of the pin) and ``i_dn_a`` (sunk into it) are read
    at ``vout_mid`` (default mid-sweep). ``output_range_v`` is the width of the
    window around ``vout_mid`` where both stay within ``tolerance`` of their
    mid values; ``mismatch_pct_max`` is the worst ``100 |I_up - I_dn| / mean``
    inside it. ``both_on_net_a_at_mid`` is the net current out of the pin with
    both on, which equals I_up - I_dn. Static only: skew, charge sharing and
    feedthrough need a transient.
    """
    mid = vout_mid if vout_mid is not None else 0.5 * (vout_min + vout_max)
    sweep = dict(Vout=slice(vout_min, vout_max, vout_step))
    states = {"up": (1, 0), "dn": (0, 1), "both": (1, 1)}

    def build(up: int, dn: int) -> Any:
        tb = ps.Testbench(dut)
        tb.V(name="out", positive=output_node, negative=ground, value=mid)
        tb.V(name="up", positive=up_node, negative=ground, value=up_levels[up])
        tb.V(name="dn", positive=dn_node, negative=ground, value=dn_levels[dn])
        _hold(tb, bias, ground)
        tb.add_dc(**sweep)
        return tb

    benches = {state: build(*on) for state, on in states.items()}

    def extract(runs: Mapping[str, Any]) -> dict[str, float]:
        v = np.asarray(runs["up"].sweep, float)
        up = np.asarray(runs["up"]["i(vout)"], float)
        dn = -np.asarray(runs["dn"]["i(vout)"], float)
        net = np.asarray(runs["both"]["i(vout)"], float)
        up_mid, dn_mid = _interp_at(v, up, mid), _interp_at(v, dn, mid)
        edges = [_nearest(crossings(v, y / y_mid, bound), mid)
                 for y, y_mid in ((up, up_mid), (dn, dn_mid)) for bound in (1 - tolerance, 1 + tolerance)]
        left = max([v[0]] + [e[0] for e in edges if not math.isnan(e[0])])
        right = min([v[-1]] + [e[1] for e in edges if not math.isnan(e[1])])
        at = np.concatenate(([left], v[(v > left) & (v < right)], [right]))
        a, b = np.interp(at, v, up), np.interp(at, v, dn)
        return {
            "i_up_a": up_mid,
            "i_dn_a": dn_mid,
            "mismatch_pct_max": float(np.max(200.0 * np.abs(a - b) / (a + b))),
            "output_range_v": float(right - left),
            "both_on_net_a_at_mid": _interp_at(v, net, mid),
        }

    return DesignBench(
        name="charge_pump_current",
        category="charge",
        testbench=benches["both"],
        intent="Charge-pump UP/DN currents vs output voltage, their mismatch and the usable output range.",
        measurements=["i_up_a", "i_dn_a", "mismatch_pct_max", "output_range_v", "both_on_net_a_at_mid"],
        result_fields=["dc.sweep", "dc['i(vout)'] per state"],
        validation=["UP/DN currents in range", "mismatch below target over the output range"],
        extractor=extract,
        runner=lambda: {state: tb.dc(**sweep) for state, tb in benches.items()},
    )
