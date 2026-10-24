"""Reusable testbench recipes for common analog and mixed-signal blocks.

Each recipe builds an ordinary ``spicerack.Testbench`` plus an extractor that
turns its result into metrics. Recipes, not a second simulation framework.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import Any, Callable

import numpy as np

from .analysis import ac_bandwidth_hz, ac_passband_db, ac_phase_margin_deg, crossings


@dataclass
class DesignBench:
    """A generated testbench plus the metrics that can be extracted from it."""

    name: str
    category: str
    testbench: Any
    intent: str
    measurements: list[str] = field(default_factory=list)
    result_fields: list[str] = field(default_factory=list)
    validation: list[str] = field(default_factory=list)
    #: Extracts ``measurements`` from a simulation result. ``None`` means the
    #: metrics in ``measurements`` are documented intent that nothing computes
    #: yet -- ``metrics()`` will say so rather than return a wrong number.
    extractor: Callable[[Any], dict[str, float]] | None = None

    def netlist(self, backend: str = "ngspice") -> str:
        return self.testbench.netlist(backend)

    @property
    def computed(self) -> bool:
        """Whether this bench can actually produce its declared metrics."""
        return self.extractor is not None

    def metrics(self, result: Any) -> dict[str, float]:
        """Extract the declared measurements from ``result``.

        Raises rather than returning partial data: a metric dict missing keys
        it advertised is worse than an error, because callers index it.
        """
        if self.extractor is None:
            raise NotImplementedError(
                f"bench '{self.name}' declares {self.measurements} as intent; "
                "no extractor is wired yet. Use tb.measure(...) or the "
                "analysis helpers directly."
            )
        values = self.extractor(result)
        missing = [m for m in self.measurements if m not in values]
        if missing:
            raise RuntimeError(f"bench '{self.name}' declared {missing} but did not compute them")
        return values


def _sample(res: Any, node: str, at: Any) -> np.ndarray:
    """``node`` linearly interpolated at time(s) ``at``."""
    return np.interp(at, np.asarray(res.time, float), np.asarray(res[node], float))


def _step_pwl(tb: Any, name: str, node: str, ref: str, levels: Any, period: float) -> None:
    """Hold ``levels[k]`` over ``[k*period, (k+1)*period)`` with period/1000 edges.

    Explicit PWL, because pulse sources with unrelated periods do not enumerate
    the states: some get visited twice and others never.
    """
    edge = period / 1000.0
    points = [(0.0, float(levels[0]))]
    for k in range(1, len(levels)):
        points += [(k * period, float(levels[k - 1])), (k * period + edge, float(levels[k]))]
    tb.PieceWiseLinearVoltageSource(name=name, positive=node, negative=ref, values=points)


def amplifier_voltage_gain(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    output_node: str = "vout",
    reference_node: str = "0",
    source_name: str = "in",
    load_resistance: float | None = None,
    ac_magnitude: float = 1.0,
    start_frequency: float = 1.0,
    stop_frequency: float = 1e9,
) -> DesignBench:
    tb = ps.Testbench(dut)
    tb.V(name=source_name, positive=input_node, negative=reference_node, value=0.0, ac=ac_magnitude)
    if load_resistance is not None:
        tb.R(name="load", positive=output_node, negative=reference_node, value=load_resistance)
    tb.save(f"V({input_node})", f"V({output_node})")
    tb.add_operating_point()
    tb.add_ac(variation="dec", number_of_points=100, start_frequency=start_frequency, stop_frequency=stop_frequency)
    # Simulator-side cross-check on the extractor below: ngspice evaluates this
    # against its own complex vectors, so a mismatch means one of the two is wrong.
    tb.measure("ac", "gain_at_1khz_db", "find", f"vdb({output_node})", "at=1k")

    def extract(ac: Any) -> dict[str, float]:
        # A sub-unity-gain stage has no 0 dB crossing, so no phase margin.
        try:
            pm = ac_phase_margin_deg(ac, output_node)
        except ValueError:
            pm = float("nan")
        return {
            "gain_midband_db": ac_passband_db(ac, output_node),
            "bandwidth_hz": ac_bandwidth_hz(ac, output_node),
            "phase_margin_deg": pm,
        }

    return DesignBench(
        name="voltage_gain",
        category="amplifier",
        testbench=tb,
        intent="Voltage-domain gain, bias, bandwidth, and load sensitivity.",
        measurements=["gain_midband_db", "bandwidth_hz", "phase_margin_deg"],
        result_fields=["op[node]", "ac.frequency", f"ac.magnitude_db('{output_node}')"],
        validation=["output bias in range", "midband gain in budget", "bandwidth above target"],
        extractor=extract,
    )


def amplifier_current_gain(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "iin",
    output_node: str = "iout",
    reference_node: str = "0",
    source_name: str = "in",
    ac_magnitude: float = 1e-6,
    load_resistance: float | None = None,
) -> DesignBench:
    tb = ps.Testbench(dut)
    tb.I(name=source_name, positive=input_node, negative=reference_node, value=0.0, ac=ac_magnitude)
    # The sense source is the output's return path to the reference: a floating
    # sense node carries no current, so the branch would read 0 for every DUT.
    if load_resistance is None:
        tb.V(name="out_sense", positive=output_node, negative=reference_node, value=0.0)
    else:
        tb.V(name="out_sense", positive=output_node, negative="out_sense_n", value=0.0)
        tb.R(name="load", positive="out_sense_n", negative=reference_node, value=load_resistance)
    tb.save("I(Vout_sense)", f"V({output_node})")
    tb.add_operating_point()
    tb.add_ac(variation="dec", number_of_points=80, start_frequency=1.0, stop_frequency=100e6)

    def extract(ac: Any) -> dict[str, float]:
        branch = "i(vout_sense)"
        return {
            "current_gain_midband": 10.0 ** (ac_passband_db(ac, branch) / 20.0) / ac_magnitude,
            "current_gain_bandwidth_hz": ac_bandwidth_hz(ac, branch),
        }

    return DesignBench(
        name="current_gain",
        category="amplifier",
        testbench=tb,
        intent="Current-domain transfer, input compliance, and output current tracking.",
        measurements=["current_gain_midband", "current_gain_bandwidth_hz"],
        result_fields=["ac.frequency", "ac.magnitude('i(vout_sense)')"],
        validation=["current gain in budget", "output compliance maintained"],
        extractor=extract,
    )


def amplifier_transimpedance(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "iin",
    output_node: str = "vout",
    reference_node: str = "0",
    source_name: str = "in",
    ac_current: float = 1e-6,
) -> DesignBench:
    tb = ps.Testbench(dut)
    tb.I(name=source_name, positive=input_node, negative=reference_node, value=0.0, ac=ac_current)
    tb.save(f"V({output_node})", f"I(V{source_name})")
    tb.add_operating_point()
    tb.add_ac(variation="dec", number_of_points=100, start_frequency=1.0, stop_frequency=1e9)

    def extract(ac: Any) -> dict[str, float]:
        # |V_out| / |I_in| in ohms; the AC source magnitude is the denominator.
        return {
            "transimpedance_ohm": 10.0 ** (ac_passband_db(ac, output_node) / 20.0) / ac_current,
            "tia_bandwidth_hz": ac_bandwidth_hz(ac, output_node),
        }

    return DesignBench(
        name="transimpedance",
        category="amplifier",
        testbench=tb,
        intent="Current-to-voltage gain, noise-relevant bandwidth, and output swing.",
        measurements=["transimpedance_ohm", "tia_bandwidth_hz"],
        result_fields=["ac.frequency", f"ac.magnitude_db('{output_node}')"],
        validation=["transimpedance in budget", "output bias in linear region"],
        extractor=extract,
    )


def charge_amplifier(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "qin",
    output_node: str = "vout",
    reference_node: str = "0",
    charge_coulombs: float = 1e-12,
    pulse_width: float = 10e-9,
) -> DesignBench:
    edge = pulse_width / 100
    period = 10 * pulse_width
    tb = ps.Testbench(dut)
    # A trapezoidal pulse carries pulsed_value * (width + (rise + fall) / 2);
    # dividing by pulse_width alone over-injects by (rise + fall) / 2.
    tb.PulseCurrentSource(
        name="qstep",
        positive=input_node,
        negative=reference_node,
        initial_value=0.0,
        pulsed_value=charge_coulombs / (pulse_width + edge),
        pulse_width=pulse_width,
        period=period,
        rise_time=edge,
        fall_time=edge,
    )
    tb.save(f"V({output_node})")
    tb.add_transient(step_time=edge, end_time=20 * pulse_width)

    def extract(tran: Any) -> dict[str, float]:
        # First pulse only (the next arrives at `period`); droop over the hold
        # window, clear of both the step and the next pulse.
        t1, t2 = 2 * pulse_width, 0.9 * period
        v_pre, v_post, v1, v2 = _sample(tran, output_node, [0.0, pulse_width + 2 * edge, t1, t2])
        return {
            "q_injected": charge_coulombs,
            "delta_vout": float(v_post - v_pre),
            "droop_rate": float((v2 - v1) / (t2 - t1)),
        }

    return DesignBench(
        name="charge_amplifier",
        category="amplifier",
        testbench=tb,
        intent="Charge-domain impulse response and droop after injected charge.",
        measurements=["q_injected", "delta_vout", "droop_rate"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["delta_vout matches Q/C target", "droop below target"],
        extractor=extract,
    )


def dac_static_linearity(
    ps: Any,
    dut: Any,
    *,
    code_nodes: list[str],
    output_node: str = "vout",
    reference_node: str = "0",
    v_high: float = 1.0,
    load_resistance: float | None = None,
    code_period: float = 1e-6,
    full_scale: float | None = None,
) -> DesignBench:
    n_codes = 2 ** len(code_nodes)
    codes = np.arange(n_codes)
    tb = ps.Testbench(dut)
    # Walk every code: one PWL per bit forming a binary counter. Stepping each
    # bit and then sweeping only bit 0 would leave the upper bits parked at zero.
    for idx, node in enumerate(code_nodes):
        _step_pwl(tb, f"code{idx}", node, reference_node, v_high * ((codes >> idx) & 1), code_period)
    if load_resistance is not None:
        tb.R(name="load", positive=output_node, negative=reference_node, value=load_resistance)
    tb.save(f"V({output_node})")
    tb.add_transient(step_time=code_period / 50.0, end_time=n_codes * code_period)

    def extract(tran: Any) -> dict[str, float]:
        # Sample each code late in its interval, after the bits have settled.
        levels = _sample(tran, output_node, (codes + 0.9) * code_period)
        lsb = (levels[-1] - levels[0]) / (n_codes - 1)
        ideal_lsb = (v_high if full_scale is None else full_scale) / (n_codes - 1)
        steps = np.diff(levels)
        inl = (levels - (levels[0] + codes * lsb)) / lsb
        return {
            "offset_error_lsb": float(levels[0] / ideal_lsb),
            "gain_error": float(lsb / ideal_lsb - 1.0),
            "dnl_max_lsb": float(np.max(np.abs(steps / lsb - 1.0))),
            "inl_max_lsb": float(np.max(np.abs(inl))),
            "monotonic": float(np.all(steps >= 0)),
        }

    return DesignBench(
        name="dac_static_linearity",
        category="dac",
        testbench=tb,
        intent="Code-to-output transfer, endpoint gain, monotonicity, DNL, and INL.",
        measurements=["offset_error_lsb", "gain_error", "dnl_max_lsb", "inl_max_lsb", "monotonic"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["monotonic output", "endpoint error in budget", "DNL/INL in budget"],
        extractor=extract,
    )


def adc_ramp(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    clock_node: str = "clk",
    output_nodes: list[str] | None = None,
    reference_node: str = "0",
    input_start: float = 0.0,
    input_stop: float = 1.0,
    conversion_period: float = 1e-6,
    conversions: int = 128,
    logic_high: float = 1.0,
) -> DesignBench:
    """Slow ramp through every code, with the digital outputs reassembled.

    ``output_nodes`` is LSB-first. Codes are rebuilt by sampling each bit late in
    its conversion period and thresholding at half the logic swing.
    """
    output_nodes = output_nodes or ["d0"]
    end_time = conversion_period * conversions
    tb = ps.Testbench(dut)
    tb.PieceWiseLinearVoltageSource(
        name="ramp",
        positive=input_node,
        negative=reference_node,
        values=[(0.0, input_start), (end_time, input_stop)],
    )
    tb.PulseVoltageSource(
        name="clk",
        positive=clock_node,
        negative=reference_node,
        initial_value=0.0,
        pulsed_value=logic_high,
        pulse_width=conversion_period / 2,
        period=conversion_period,
        rise_time=conversion_period / 1000,
        fall_time=conversion_period / 1000,
    )
    tb.save(f"V({input_node})", f"V({clock_node})", *(f"V({node})" for node in output_nodes))
    tb.add_transient(step_time=conversion_period / 50, end_time=end_time)

    def extract(tran: Any) -> dict[str, float]:
        t = (np.arange(conversions) + 0.9) * conversion_period
        bits = np.array([_sample(tran, node, t) > logic_high / 2.0 for node in output_nodes])
        codes = (bits.astype(int) << np.arange(len(output_nodes))[:, None]).sum(axis=0)
        steps = np.diff(codes)
        # Missing codes are counted over full scale, the conventional definition,
        # so a partial ramp reports codes it never reached as missing; compare
        # `codes_seen` against 2**n_bits to tell the two apart.
        seen = len(np.unique(codes))
        return {
            "codes_seen": float(seen),
            "missing_codes": float((1 << len(output_nodes)) - seen),
            "transition_count": float(np.count_nonzero(steps)),
            "code_monotonic": float(np.all(steps >= 0)),
        }

    return DesignBench(
        name="adc_ramp",
        category="adc",
        testbench=tb,
        intent="Ramp-code coverage, missing codes and code monotonicity.",
        # Latency needs a DUT with a known pipeline depth to align against.
        measurements=["codes_seen", "missing_codes", "transition_count", "code_monotonic"],
        result_fields=["tran.time", f"tran['{input_node}']", "digital output nodes"],
        validation=["codes increase monotonically", "no missing codes in target range"],
        extractor=extract,
    )


def switch_characterization(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    output_node: str = "vout",
    control_node: str = "ctrl",
    reference_node: str = "0",
    load_resistance: float = 1e3,
    signal_voltage: float = 1.0,
    control_high: float = 1.0,
) -> DesignBench:
    """On- and off-resistance from a control sweep across a known load.

    The load is what makes the switch observable: into a capacitor-only load no
    DC current flows, V(out) equals V(in) for any on-resistance.
    """
    tb = ps.Testbench(dut)
    tb.V(name="sig", positive=input_node, negative=reference_node, value=signal_voltage)
    tb.V(name="ctrl", positive=control_node, negative=reference_node, value=0.0)
    tb.R(name="load", positive=output_node, negative=reference_node, value=load_resistance)
    tb.save(f"V({input_node})", f"V({output_node})", f"V({control_node})")
    tb.add_dc(Vctrl=slice(0.0, control_high, control_high / 20.0))

    def extract(dc: Any) -> dict[str, float]:
        vout = dc[output_node]
        v_off, v_on = float(vout[0]), float(vout[-1])
        # Divider: Vout = Vin * RL / (RL + Rsw)  ->  Rsw = RL * (Vin/Vout - 1)
        r_sw = lambda v: load_resistance * (signal_voltage / v - 1.0) if v > 0.0 else math.inf
        return {
            "ron_ohm": r_sw(v_on),
            "roff_ohm": r_sw(v_off),
            "off_isolation_db": 20.0 * math.log10(v_off / signal_voltage) if v_off > 0.0 else -math.inf,
        }

    return DesignBench(
        name="switch_characterization",
        category="switch",
        testbench=tb,
        intent="On-resistance, off-resistance and DC off isolation across a stated load.",
        # Charge injection needs a transistor-level switch: a behavioural ngspice
        # S/W element carries no channel charge and would report exactly zero.
        measurements=["ron_ohm", "roff_ohm", "off_isolation_db"],
        result_fields=["dc.sweep", f"dc['{output_node}']"],
        validation=["on path conducts", "off path stays isolated"],
        extractor=extract,
    )


def _one_hot_walk(tb: Any, select_nodes: list[str], reference_node: str,
                  v_high: float, phase: float) -> None:
    """Drive select lines one-hot, one channel per phase."""
    hot = np.eye(len(select_nodes)) * v_high
    for idx, node in enumerate(select_nodes):
        _step_pwl(tb, f"sel{idx}", node, reference_node, hot[idx], phase)


def mux_routing(
    ps: Any,
    dut: Any,
    *,
    input_nodes: list[str],
    output_node: str = "vout",
    select_nodes: list[str] | None = None,
    reference_node: str = "0",
    load_resistance: float = 1e3,
    v_high: float = 1.0,
    phase: float = 1e-6,
) -> DesignBench:
    """One-hot channel walk with a stated output load.

    Without a load the output follows the selected input exactly regardless of
    on-resistance, so path gain and channel matching are unobservable.
    """
    select_nodes = select_nodes or [f"sel{i}" for i in range(len(input_nodes))]
    levels = np.arange(1, len(input_nodes) + 1, dtype=float)
    tb = ps.Testbench(dut)
    for idx, node in enumerate(input_nodes):
        tb.V(name=f"in{idx}", positive=node, negative=reference_node, value=float(levels[idx]))
    _one_hot_walk(tb, select_nodes, reference_node, v_high, phase)
    tb.R(name="load", positive=output_node, negative=reference_node, value=load_resistance)
    tb.save(f"V({output_node})", *(f"V({n})" for n in input_nodes + select_nodes))
    tb.add_transient(step_time=phase / 100.0, end_time=len(input_nodes) * phase)

    def extract(tran: Any) -> dict[str, float]:
        v = _sample(tran, output_node, (np.arange(len(levels)) + 0.9) * phase)
        gains = v / levels
        # The output must sit nearer its own channel than any other.
        nearest = np.argmin(np.abs(v[:, None] - levels[None, :]), axis=1)
        return {
            "selected_gain_min": float(gains.min()),
            "selected_gain_max": float(gains.max()),
            "channel_gain_mismatch": float(np.ptp(gains)),
            "all_channels_routed": float(np.all(nearest == np.arange(len(levels)))),
        }

    return DesignBench(
        name="mux_routing",
        category="mux",
        testbench=tb,
        intent="One-hot selection, selected-path gain and channel-to-channel matching.",
        measurements=["selected_gain_min", "selected_gain_max",
                      "channel_gain_mismatch", "all_channels_routed"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["one selected path at a time", "output equals selected input within tolerance"],
        extractor=extract,
    )


def demux_routing(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    output_nodes: list[str],
    select_nodes: list[str] | None = None,
    reference_node: str = "0",
    load_resistance: float = 1e3,
    input_voltage: float = 1.0,
    v_high: float = 1.0,
    phase: float = 1e-6,
) -> DesignBench:
    """One input routed to one output per phase, every output loaded."""
    select_nodes = select_nodes or [f"sel{i}" for i in range(len(output_nodes))]
    tb = ps.Testbench(dut)
    tb.V(name="in", positive=input_node, negative=reference_node, value=input_voltage)
    _one_hot_walk(tb, select_nodes, reference_node, v_high, phase)
    for idx, node in enumerate(output_nodes):
        tb.R(name=f"load{idx}", positive=node, negative=reference_node, value=load_resistance)
    tb.save(f"V({input_node})", *(f"V({n})" for n in output_nodes + select_nodes))
    tb.add_transient(step_time=phase / 100.0, end_time=len(output_nodes) * phase)

    def extract(tran: Any) -> dict[str, float]:
        n = len(output_nodes)
        t = (np.arange(n) + 0.9) * phase
        # gain[k, phase]: output k during the phase that selects channel `phase`.
        gain = np.array([_sample(tran, node, t) for node in output_nodes]) / input_voltage
        off = gain[~np.eye(n, dtype=bool)]
        off_max = float(off.max()) if off.size else 0.0
        return {
            "selected_gain_min": float(np.diag(gain).min()),
            "inactive_output_max": off_max,
            "off_isolation_db": 20.0 * math.log10(off_max) if off_max > 0.0 else -math.inf,
        }

    return DesignBench(
        name="demux_routing",
        category="demux",
        testbench=tb,
        intent="One input routed to one loaded output while inactive outputs stay isolated.",
        measurements=["selected_gain_min", "inactive_output_max", "off_isolation_db"],
        result_fields=["tran.time", "output node waveforms"],
        validation=["only selected output follows input", "inactive outputs stay bounded"],
        extractor=extract,
    )


def sample_hold(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    output_node: str = "vhold",
    clock_node: str = "phi",
    reference_node: str = "0",
    input_frequency: float = 1e3,
    hold_capacitance: float = 1e-12,
    sample_frequency: float | None = None,
    clock_edge: float | None = None,
) -> DesignBench:
    # The clock must run well above the input, or the sampler catches the same
    # input phase every cycle and the record is DC.
    period = 1.0 / (sample_frequency or 20.0 * input_frequency)
    edge = clock_edge if clock_edge is not None else period / 1000.0

    tb = ps.Testbench(dut)
    tb.SinusoidalVoltageSource(
        name="in",
        positive=input_node,
        negative=reference_node,
        offset=0.5,
        amplitude=0.5,
        frequency=input_frequency,
    )
    tb.PulseVoltageSource(
        name="phi",
        positive=clock_node,
        negative=reference_node,
        initial_value=0.0,
        pulsed_value=1.0,
        pulse_width=period / 2.0,
        period=period,
        rise_time=edge,
        fall_time=edge,
    )
    tb.C(name="hold", positive=output_node, negative=reference_node, value=hold_capacitance)
    tb.save(f"V({input_node})", f"V({output_node})", f"V({clock_node})")
    # Resolve the track->hold edge, not the input period.
    tb.add_transient(step_time=period / 500.0, end_time=2.0 / input_frequency)

    def extract(tran: Any) -> dict[str, float]:
        # A late clock period, past start-up; the clock falls at mid-period.
        t_fall = (int(tran.time[-1] / period) - 1) * period + period / 2.0
        t1, t2 = t_fall + 10.0 * edge, t_fall + 0.45 * period
        v_track, v1, v2 = _sample(tran, output_node, [t_fall - edge, t1, t2])
        return {
            "acquisition_error": float(v_track - _sample(tran, input_node, t_fall)),
            "hold_droop": float((v2 - v1) / (t2 - t1)),
            "pedestal_step": float(v1 - v_track),
        }

    return DesignBench(
        name="sample_hold",
        category="sample_hold",
        testbench=tb,
        intent="Acquisition error, hold droop, and pedestal at the track-to-hold edge.",
        # Aperture delay needs two runs with opposite input slopes to separate it
        # from the pedestal; it is not observable from this single record.
        measurements=["acquisition_error", "hold_droop", "pedestal_step"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["settles during track", "droop during hold below target"],
        extractor=extract,
    )


def pll_lock(
    ps: Any,
    dut: Any,
    *,
    reference_node: str = "ref",
    output_node: str = "vco",
    control_node: str = "vctrl",
    ground: str = "0",
    reference_frequency: float = 10e6,
    divide_ratio: float = 1.0,
    cycles: int = 256,
) -> DesignBench:
    """Reference-driven transient; output frequency and control settling.

    Phase noise and reference spurs are not reachable here: ngspice injects no
    device noise into a transient, and the `.four` table it would need is not
    parsed. Deterministic quantities -- frequency, control settling -- are.
    """
    period = 1.0 / reference_frequency
    tb = ps.Testbench(dut)
    tb.PulseVoltageSource(
        name="ref",
        positive=reference_node,
        negative=ground,
        initial_value=0.0,
        pulsed_value=1.0,
        pulse_width=period / 2,
        period=period,
        rise_time=period / 100,
        fall_time=period / 100,
    )
    tb.save(f"V({reference_node})", f"V({output_node})", f"V({control_node})")
    tb.add_transient(step_time=period / 200, end_time=period * cycles)

    def extract(tran: Any) -> dict[str, float]:
        time = np.asarray(tran.time, float)
        vco = np.asarray(tran[output_node], float)
        vctrl = np.asarray(tran[control_node], float)
        settle_from = 0.5 * time[-1]
        # Mean frequency over every rising edge in the second half: one period
        # alone inherits the timestep quantisation of its two edges.
        edges = [x for x in crossings(time, vco, (vco.max() + vco.min()) / 2.0, rising=True)
                 if x >= settle_from]
        if len(edges) < 2:
            raise ValueError(f"need at least 2 rising crossings after {settle_from:g}s, got {len(edges)}")
        f_out = (len(edges) - 1) / (edges[-1] - edges[0])
        tail = vctrl[time >= settle_from]
        return {
            "output_frequency_hz": f_out,
            "frequency_error_ratio": f_out / (reference_frequency * divide_ratio) - 1.0,
            "control_settling_v": float(np.ptp(tail)) if tail.size else math.nan,
            "control_final_v": float(vctrl[-1]),
        }

    return DesignBench(
        name="pll_lock",
        category="pll",
        testbench=tb,
        intent="Output frequency against the reference, and control-node settling.",
        measurements=["output_frequency_hz", "frequency_error_ratio",
                      "control_settling_v", "control_final_v"],
        result_fields=["tran.time", f"tran['{control_node}']", f"tran['{output_node}']"],
        validation=["control voltage settles", "output period matches reference ratio"],
        extractor=extract,
    )


def bandgap_reference(
    ps: Any,
    dut: Any,
    *,
    supply_node: str = "vdd",
    output_node: str = "vref",
    ground: str = "0",
    supply_start: float = 1.0,
    supply_stop: float = 5.0,
) -> DesignBench:
    tb = ps.Testbench(dut)
    tb.V(name="dd", positive=supply_node, negative=ground, value=supply_stop)
    tb.save(f"V({supply_node})", f"V({output_node})")
    tb.temperature = 27.0
    tb.nominal_temperature = 27.0
    # Sweep only the in-regulation window; below dropout the "reference" is just
    # tracking the rail, and a slope fitted across that is a dropout measurement.
    tb.add_dc(Vdd=slice(supply_start, supply_stop, (supply_stop - supply_start) / 20))

    def extract(dc: Any) -> dict[str, float]:
        vref = dc[output_node]
        sweep = dc.sweep
        v_lo, v_hi = float(vref[0]), float(vref[-1])
        return {
            "vref": (v_lo + v_hi) / 2.0,
            "line_regulation_ppm_v": 1e6 * (v_hi - v_lo) / (v_lo * (sweep[-1] - sweep[0])),
        }

    return DesignBench(
        name="bandgap_reference",
        category="bandgap",
        testbench=tb,
        intent="Output level and line regulation across the in-regulation supply window.",
        # Temperature drift needs a `.dc temp` sweep (bandgap_tempco) and startup
        # needs a transient from a genuinely off state; neither is observable here.
        measurements=["vref", "line_regulation_ppm_v"],
        result_fields=["dc.sweep", f"dc['{output_node}']"],
        validation=["vref in target range", "line regulation below target"],
        extractor=extract,
    )


def bandgap_tempco(
    ps: Any,
    dut: Any,
    *,
    supply_node: str = "vdd",
    output_node: str = "vref",
    ground: str = "0",
    supply: float = 5.0,
    t_min: float = -40.0,
    t_max: float = 125.0,
    t_step: float = 5.0,
) -> DesignBench:
    """Temperature drift via `.dc temp`, which every backend supports.

    `.step param temp` is emitted commented out by the ngspice codegen, so a
    step-based sweep silently runs at one temperature.
    """
    tb = ps.Testbench(dut)
    tb.V(name="dd", positive=supply_node, negative=ground, value=supply)
    tb.save(f"V({output_node})")
    tb.nominal_temperature = 27.0
    tb.add_dc(temp=slice(t_min, t_max, t_step))

    def extract(dc: Any) -> dict[str, float]:
        temps = np.asarray(dc.sweep, float)
        vref = np.asarray(dc[output_node], float)
        v27 = float(np.interp(27.0, temps, vref))
        scale = 1e6 / (v27 * (temps[-1] - temps[0]))
        return {
            "vref_27c": v27,
            # Box method: the industry default, and what datasheets guarantee.
            "tempco_box_ppm_c": float(np.ptp(vref) * scale),
            # Endpoint method, reported alongside: the two disagree on a curved
            # response and datasheets do not always say which they used.
            "tempco_endpoint_ppm_c": float((vref[-1] - vref[0]) * scale),
        }

    return DesignBench(
        name="bandgap_tempco",
        category="bandgap",
        testbench=tb,
        intent="Reference drift over temperature, box and endpoint conventions.",
        measurements=["vref_27c", "tempco_box_ppm_c", "tempco_endpoint_ppm_c"],
        result_fields=["dc.sweep", f"dc['{output_node}']"],
        validation=["tempco below target", "vref at 27C in range"],
        extractor=extract,
    )
