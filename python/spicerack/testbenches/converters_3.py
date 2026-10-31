"""Architecture-specific converter benches: comparator kickback, pipeline
stage residue and DAC output impedance."""

from __future__ import annotations

import math
from typing import Any

import numpy as np

from .analysis import ac_bandwidth_hz
from .characterize import _integral
from .converters import _clock, _transient
from .design import DesignBench, _step_pwl


def comparator_kickback(
    ps: Any,
    dut: Any,
    *,
    inp: str = "inp",
    inn: str = "inn",
    clock_node: str = "clk",
    reference_node: str = "0",
    vcm: float = 0.5,
    overdrive: float = 10e-3,
    source_resistance_p: float = 10e3,
    source_resistance_n: float = 100.0,
    source_capacitance: float = 200e-15,
    clock_period: float = 10e-9,
    cycles: int = 8,
    logic_high: float = 1.0,
    max_step: float | None = None,
) -> DesignBench:
    """Charge the comparator kicks into non-ideal drivers, and the input error
    it leaves at the next decision.

    Each input is driven from an ideal source (``<input>_src``) through its
    source resistance, with ``source_capacitance`` to the reference at the pin;
    ideal sources at the pins would absorb every kick and read zero. Unequal
    resistances turn a common-mode kick into a differential error. The
    differential source alternates +-``overdrive`` per cycle; clock as in
    ``comparator_threshold`` (reset low from kT, evaluate high from (k + 1/2) T).

    ``kick_charge_c``: largest |charge| either source delivers over the half
    period after a clock edge. ``kick_peak_v``: largest |V(inp) - V(inp_src)|.
    ``kick_diff_peak_v``: largest differential pin error. ``kick_residual_v``:
    the differential pin error at the last two nominal rising (sampling) edges,
    one per input polarity, before the clock moves: what corrupts the next
    decision once ``cycles`` of history have built up. ``max_step`` (default
    T/500) must resolve the faster source time constant R_s (C_s + C_in).
    """
    vd = overdrive * (1 - 2 * (np.arange(cycles) % 2))
    srcp, srcn = f"{inp}_src", f"{inn}_src"
    tb = ps.Testbench(dut)
    _step_pwl(tb, "srcp", srcp, reference_node, vcm + vd / 2, clock_period)
    _step_pwl(tb, "srcn", srcn, reference_node, vcm - vd / 2, clock_period)
    for name, src, pin, r in (("p", srcp, inp, source_resistance_p), ("n", srcn, inn, source_resistance_n)):
        tb.R(name=f"s{name}", positive=src, negative=pin, value=r)
        tb.C(name=f"s{name}", positive=pin, negative=reference_node, value=source_capacitance)
    _clock(tb, clock_node, reference_node, clock_period, logic_high, reset_first=True)
    tb.save(*(f"V({n})" for n in (inp, inn, srcp, srcn)), "I(Vsrcp)", "I(Vsrcn)")
    runner = _transient(tb, clock_period / 1000, cycles * clock_period, max_step or clock_period / 500)

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        v = {n: np.asarray(tran[n], float) for n in (inp, inn, srcp, srcn)}
        err = (v[inp] - v[inn]) - (v[srcp] - v[srcn])
        edges = np.arange(2 * cycles) * clock_period / 2
        charge = max(abs(_integral(t, np.asarray(tran[i], float), e, e + clock_period / 2))
                     for i in ("i(vsrcp)", "i(vsrcn)") for e in edges[edges + clock_period / 2 <= t[-1]])
        return {
            "kick_charge_c": charge,
            "kick_peak_v": float(np.abs(v[inp] - v[srcp]).max()),
            "kick_diff_peak_v": float(np.abs(err).max()),
            "kick_residual_v": float(np.abs(np.interp(edges[1::2][-2:], t, err)).max()),
        }

    return DesignBench(
        name="comparator_kickback",
        category="comparator",
        testbench=tb,
        intent="Kickback charge into resistive drivers and the residual input error at sampling.",
        measurements=["kick_charge_c", "kick_peak_v", "kick_diff_peak_v", "kick_residual_v"],
        result_fields=["tran.time", "i(vsrcp)", "i(vsrcn)", f"tran['{inp}']", f"tran['{inn}']"],
        validation=["residual below a fraction of an LSB at the sampling edge"],
        extractor=extract,
        runner=runner,
    )


def _phase_pwl(tb: Any, name: str, node: str, reference_node: str, start: float, stop: float,
               period: float, cycles: int, high: float) -> None:
    """High over [kT + start*T, kT + stop*T] for every cycle, edges T/1000."""
    edge = period / 1000
    points = [(0.0, 0.0)]
    for k in range(cycles):
        a, b = (k + start) * period, (k + stop) * period
        points += [(a, 0.0), (a + edge, high), (b, high), (b + edge, 0.0)]
    tb.PieceWiseLinearVoltageSource(name=name, positive=node, negative=reference_node, values=points)


def pipeline_stage_residue(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    residue_node: str = "vres",
    phi1_node: str = "phi1",
    phi2_node: str = "phi2",
    reference_node: str = "0",
    vref: float = 0.5,
    stage_bits: float = 1.5,
    levels: int = 257,
    conversion_period: float = 100e-9,
    dead_time: float | None = None,
    logic_high: float = 1.0,
) -> DesignBench:
    """Interstage gain, residue range and sub-ADC threshold errors of one
    pipeline stage (MDAC) with redundancy (``stage_bits`` 1.5, 2.5, ...).

    The input steps through ``levels`` values across +-``vref``, one per
    conversion. phi1 (sample) is high over the first half of each period and
    phi2 (amplify) over the second, each shortened by ``dead_time`` (default
    T/50) at both ends; the residue is read T/100 before phi2 falls.

    A threshold is the midpoint of the two levels around a residue drop of
    more than vref/2; ``threshold_error_max_v`` is against the ideal
    (2i + 1) vref / 2^(m + 1) of an (m + 0.5)-bit stage, exact to half a level
    step. ``interstage_gain`` is the median of per-segment slopes, ideal
    2^m; ``residue_jump_v`` the mean drop with the slope removed, ideal vref.
    """
    half = round(2 ** (stage_bits - 0.5))
    ideal = (2 * np.arange(-(half - 1), half - 1) + 1) * vref / (2 * half)
    v = np.linspace(-vref, vref, levels)
    period = conversion_period
    dead = (dead_time if dead_time is not None else period / 50) / period
    tb = ps.Testbench(dut)
    _step_pwl(tb, "in", input_node, reference_node, v, period)
    _phase_pwl(tb, "phi1", phi1_node, reference_node, dead, 0.5 - dead, period, levels, logic_high)
    _phase_pwl(tb, "phi2", phi2_node, reference_node, 0.5 + dead, 1 - dead, period, levels, logic_high)
    tb.save(f"V({input_node})", f"V({residue_node})")
    runner = _transient(tb, period / 100, levels * period, period / 50)

    def extract(tran: Any) -> dict[str, float]:
        r = np.interp((np.arange(levels) + 1 - dead - 0.01) * period, np.asarray(tran.time, float),
                      np.asarray(tran[residue_node], float))
        dr = np.diff(r)
        jumps = np.nonzero(dr < -vref / 2)[0]
        if len(jumps) != len(ideal):
            raise ValueError(f"found {len(jumps)} residue jumps, a {stage_bits}-bit stage has {len(ideal)}")
        bounds = np.concatenate([[0], jumps + 1, [levels]])
        gain = float(np.median([np.polyfit(v[a:b], r[a:b], 1)[0] for a, b in zip(bounds[:-1], bounds[1:])
                                if b - a >= 2]))
        peak = float(np.abs(r).max())
        return {
            "interstage_gain": gain,
            "gain_error": gain / half - 1,
            "residue_max_v": peak,
            "overrange_margin_v": vref - peak,
            "threshold_error_max_v": float(np.abs((v[jumps] + v[jumps + 1]) / 2 - ideal).max()),
            "residue_jump_v": float(np.mean(gain * (v[1] - v[0]) - dr[jumps])),
        }

    return DesignBench(
        name="pipeline_stage_residue",
        category="adc",
        testbench=tb,
        intent="One MDAC stage's transfer: interstage gain, residue range, threshold errors.",
        measurements=["interstage_gain", "gain_error", "residue_max_v", "overrange_margin_v",
                      "threshold_error_max_v", "residue_jump_v"],
        result_fields=["tran.time", f"tran['{residue_node}']"],
        validation=["residue stays inside +-vref", "threshold errors inside the redundancy"],
        extractor=extract,
        runner=runner,
    )


def dac_output_impedance(
    ps: Any,
    dut: Any,
    *,
    code_nodes: list[str],
    code: int,
    output_node: str = "vout",
    reference_node: str = "0",
    v_high: float = 1.0,
    load_resistance: float = 25.0,
    f_spec: float = 100e6,
    start_frequency: float = 1e3,
    stop_frequency: float = 10e9,
) -> DesignBench:
    """Output impedance at one static ``code``, loaded: |Z| = V(out) for 1 A AC
    driven into the output. One bench per code; loop over 0, 1, 2^(N-1) and
    the top code, since a code-dependent R_out is what bends the INL and
    C_out the high-frequency SFDR. ``zout_corner_hz`` is where |Z| falls
    3.01 dB under its low-frequency value, inf when the sweep never gets there.
    """
    tb = ps.Testbench(dut)
    for idx, node in enumerate(code_nodes):
        tb.V(name=f"code{idx}", positive=node, negative=reference_node, value=v_high * ((code >> idx) & 1))
    tb.R(name="load", positive=output_node, negative=reference_node, value=load_resistance)
    tb.I(name="z", positive=reference_node, negative=output_node, value=0.0, ac=1.0)
    tb.save(f"V({output_node})")
    sweep = dict(variation="dec", number_of_points=50, start_frequency=start_frequency,
                 stop_frequency=stop_frequency)
    tb.add_ac(**sweep)

    def extract(ac: Any) -> dict[str, float]:
        f = np.asarray(ac.frequency, float)
        z = np.asarray(ac.magnitude(output_node), float)
        try:
            corner = ac_bandwidth_hz(ac, output_node, reference_db=20 * math.log10(z[0]))
        except ValueError:
            corner = math.inf
        return {"zout_dc_ohm": float(z[0]),
                "zout_at_f_ohm": float(np.exp(np.interp(math.log(f_spec), np.log(f), np.log(z)))),
                "zout_corner_hz": corner}

    return DesignBench(
        name="dac_output_impedance",
        category="dac",
        testbench=tb,
        intent="Code-dependent output impedance of a current-steering DAC under its load.",
        measurements=["zout_dc_ohm", "zout_at_f_ohm", "zout_corner_hz"],
        result_fields=["ac.frequency", f"ac.magnitude('{output_node}')"],
        validation=["Z_out high enough for the INL budget", "corner above the signal band"],
        extractor=extract,
        runner=lambda: tb.ac(**sweep),
    )
