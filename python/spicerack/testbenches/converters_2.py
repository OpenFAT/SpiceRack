"""DAC and sample-and-hold testbenches: transition settling and glitch, DAC
spectrum, sampled-sine spectrum, track bandwidth and hold feedthrough.

Shared machinery (``coherent_bin``, the spectrum core) lives in ``converters``.
"""

from __future__ import annotations

import math
from typing import Any

import numpy as np

from ..remeasure import _settling, _slew
from .analysis import ac_bandwidth_hz, ac_passband_db, crossings
from .characterize import _integral
from .converters import _clock, _tone, _transient, coherent_bin
from .design import DesignBench, _step_pwl


def _codes_pwl(tb: Any, code_nodes: list[str], reference_node: str, codes: np.ndarray,
               v_high: float, period: float) -> None:
    """One PWL per bit (LSB first) holding ``codes[k]`` over [kT, (k+1)T)."""
    for idx, node in enumerate(code_nodes):
        _step_pwl(tb, f"code{idx}", node, reference_node, v_high * ((codes >> idx) & 1), period)


def _loads(tb: Any, node: str, reference_node: str, resistance: float | None,
           capacitance: float | None) -> None:
    if resistance is not None:
        tb.R(name="load", positive=node, negative=reference_node, value=resistance)
    if capacitance is not None:
        tb.C(name="load", positive=node, negative=reference_node, value=capacitance)


def dac_transition(
    ps: Any,
    dut: Any,
    *,
    code_nodes: list[str],
    output_node: str = "vout",
    reference_node: str = "0",
    v_high: float = 1.0,
    hold_time: float = 1e-6,
    load_resistance: float | None = None,
    load_capacitance: float | None = None,
    settle_band_lsb: float = 0.5,
    max_step: float | None = None,
) -> DesignBench:
    """Settling, delay, slew and major-carry glitch impulse in one record.

    Codes [c-1, c, c-1, 0, max, 0], c = 2^(N-1), each held ``hold_time``; edge
    k starts at t_e = k * hold_time and takes hold_time/1000. Each final value
    is the mean over the last 10 % of its hold; the LSB is the full-scale step
    over 2^N - 1.

    Settling runs from t_e to the last exit of +-``settle_band_lsb`` around the
    final value, worst of the full-scale rise and fall (``settling_time_s``) and
    of the two carry steps (``settling_time_carry_s``). ``delay_s`` is t_e to the
    50 % crossing of the full-scale rise; ``slew_rate_v_per_s`` the slower 10-90 %
    rate of the full-scale rise and fall.

    Glitch (V*s, never "energy"): the reference is the ideal step from the old
    to the new final value at t_e + ``delay_s``, integrated over [t_e, t_e +
    hold/2]. ``glitch_impulse_vs`` is the net area on the rising carry c-1 -> c;
    ``glitch_impulse_abs_vs`` the larger |area| integral and ``glitch_peak_v`` the
    larger peak deviation of the two carries. Clock feedthrough counts.
    """
    n_bits = len(code_nodes)
    top = 2 ** n_bits - 1
    carry = 2 ** (n_bits - 1)
    codes = np.array([carry - 1, carry, carry - 1, 0, top, 0])
    tb = ps.Testbench(dut)
    _codes_pwl(tb, code_nodes, reference_node, codes, v_high, hold_time)
    _loads(tb, output_node, reference_node, load_resistance, load_capacitance)
    tb.save(f"V({output_node})")
    runner = _transient(tb, hold_time / 1000, len(codes) * hold_time, max_step or hold_time / 1000)

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        y = np.asarray(tran[output_node], float)
        final = [float(np.mean(np.interp(np.linspace(k + 0.9, k + 1, 32, endpoint=False) * hold_time, t, y)))
                 for k in range(len(codes))]
        lsb = (final[4] - final[3]) / top

        def window(k: int) -> tuple[np.ndarray, np.ndarray]:
            keep = (t >= (k - 0.001) * hold_time) & (t < (k + 0.999) * hold_time)
            return t[keep], y[keep]

        def settle(k: int) -> float:
            tk, yk = window(k)
            return _settling(tk, yk, final[k], t0=k * hold_time, abs_tol=settle_band_lsb * lsb)

        t_rise, y_rise = window(4)
        delay = crossings(t_rise, y_rise, (final[3] + final[4]) / 2, rising=True)[0] - 4 * hold_time
        t_fall, y_fall = window(5)
        slew = min(_slew(t_rise, y_rise)["rising"], _slew(t_fall, y_fall)["falling"])

        def glitch(k: int) -> tuple[float, float, float]:
            te, ts, end = k * hold_time, k * hold_time + delay, (k + 0.5) * hold_time
            net = _integral(t, y, te, end) - final[k - 1] * delay - final[k] * (end - ts)
            area = _integral(t, np.abs(y - final[k - 1]), te, ts) + _integral(t, np.abs(y - final[k]), ts, end)
            inside = (t >= te) & (t <= end)
            peak = np.abs(y[inside] - np.where(t[inside] < ts, final[k - 1], final[k])).max()
            y_ts = float(np.interp(ts, t, y))  # the reference jumps at ts, between solver points
            return net, area, max(float(peak), abs(y_ts - final[k - 1]), abs(y_ts - final[k]))

        up, down = glitch(1), glitch(2)
        return {
            "settling_time_s": max(settle(4), settle(5)),
            "settling_time_carry_s": max(settle(1), settle(2)),
            "delay_s": float(delay),
            "slew_rate_v_per_s": float(slew),
            "glitch_impulse_vs": up[0],
            "glitch_impulse_abs_vs": max(up[1], down[1]),
            "glitch_peak_v": max(up[2], down[2]),
        }

    return DesignBench(
        name="dac_transition",
        category="dac",
        testbench=tb,
        intent="Full-scale settling, delay and slew; major-carry glitch impulse.",
        measurements=["settling_time_s", "settling_time_carry_s", "delay_s", "slew_rate_v_per_s",
                      "glitch_impulse_vs", "glitch_impulse_abs_vs", "glitch_peak_v"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["settles within the update period", "glitch impulse in budget"],
        extractor=extract,
        runner=runner,
    )


def dac_dynamic(
    ps: Any,
    dut: Any,
    *,
    code_nodes: list[str],
    output_node: str = "vout",
    reference_node: str = "0",
    v_high: float = 1.0,
    full_scale: float = 1.0,
    update_period: float = 10e-9,
    record: int = 1024,
    target_fraction: float = 0.1,
    amplitude_dbfs: float = 0.0,
    samples_per_update: int = 8,
    settle_updates: int = 8,
    load_resistance: float | None = None,
    load_capacitance: float | None = None,
    harmonics: int = 5,
) -> DesignBench:
    """SFDR, THD and SNDR of a DAC playing a coherent sine code sequence.

    Codes c[n] = round((2^N - 1)/2 * (1 + a sin(2 pi J n / record))), a from
    ``amplitude_dbfs``. The output is read ``samples_per_update`` (K) times per
    update, at (n + (j + 1/2)/K) T, and the spectrum is reported over the first
    Nyquist zone of the update rate, harmonics folded there. K = 1 reads only
    the settled value (the DAC inside an ADC); K > 1 sees glitches and
    incomplete settling, and its fundamental carries the hold (sinc) droop.
    ``fundamental_dbfs`` is against ``full_scale``, the output span from code 0
    to the top code. The load is part of the spec: 25 ohm for a
    double-terminated current-steering DAC.
    """
    levels = 2 ** len(code_nodes)
    j = coherent_bin(record, target_fraction, harmonics)
    n = np.arange(settle_updates + record)
    a = 10 ** (amplitude_dbfs / 20)
    codes = np.round((levels - 1) / 2 * (1 + a * np.sin(2 * np.pi * j * n / record))).astype(np.int64)
    k = samples_per_update
    tb = ps.Testbench(dut)
    _codes_pwl(tb, code_nodes, reference_node, codes, v_high, update_period)
    _loads(tb, output_node, reference_node, load_resistance, load_capacitance)
    tb.save(f"V({output_node})")
    runner = _transient(tb, update_period / 100, len(n) * update_period, update_period / (10 * k))

    def extract(tran: Any) -> dict[str, float]:
        reads = settle_updates + (np.arange(record * k) + 0.5) / k
        x = np.interp(reads * update_period, np.asarray(tran.time, float), np.asarray(tran[output_node], float))
        m = _tone(x, j, harmonics, band=record // 2, alias=record)
        return m | {"fundamental_dbfs": 20 * math.log10(m.pop("amplitude") / (full_scale / 2))}

    return DesignBench(
        name="dac_dynamic",
        category="dac",
        testbench=tb,
        intent="Spectral purity of the DAC output for a coherent sine code sequence.",
        measurements=["sfdr_dbc", "thd_db", "sndr_db", "snr_db", "hd2_db", "hd3_db", "enob_bits",
                      "fundamental_dbfs"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["SFDR above target at stated amplitude and fin"],
        extractor=extract,
        runner=runner,
    )


def sample_hold_dynamic(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    output_node: str = "vhold",
    clock_node: str = "phi",
    reference_node: str = "0",
    hold_capacitance: float = 1e-12,
    sample_frequency: float = 10e6,
    record: int = 1024,
    target_fraction: float = 0.45,
    offset: float = 0.5,
    amplitude: float = 0.45,
    read_fraction: float = 0.9,
    settle_cycles: int = 8,
    harmonics: int = 5,
    reltol: float = 1e-5,
) -> DesignBench:
    """SNDR, SFDR, THD and sampled gain of a track-and-hold from held values only.

    The clock (as in ``sample_hold``) tracks while high over the first half
    period and holds from (k + 1/2) T; the output is read at
    (k + 1/2 + read_fraction/2) T, late in the hold, after ``settle_cycles``.
    ``sampled_gain_db`` is the held fundamental against the source amplitude.
    The default fin sits near Nyquist, where tracking distortion is worst;
    run 0.1 too. Report amplitude with the result: HD3 grows as A^2.
    """
    period = 1 / sample_frequency
    j = coherent_bin(record, target_fraction, harmonics)
    fin = j * sample_frequency / record
    tb = ps.Testbench(dut)
    tb.SinusoidalVoltageSource(name="in", positive=input_node, negative=reference_node,
                               offset=offset, amplitude=amplitude, frequency=fin)
    _clock(tb, clock_node, reference_node, period, 1.0)
    tb.C(name="hold", positive=output_node, negative=reference_node, value=hold_capacitance)
    tb.options(reltol=reltol)
    tb.save(f"V({input_node})", f"V({output_node})")
    # The track phase is a moving node: resolve the input to 1/200 of its period.
    runner = _transient(tb, period / 200, (settle_cycles + record) * period, min(period / 200, 1 / (200 * fin)))

    def extract(tran: Any) -> dict[str, float]:
        reads = (settle_cycles + np.arange(record) + 0.5 + read_fraction / 2) * period
        x = np.interp(reads, np.asarray(tran.time, float), np.asarray(tran[output_node], float))
        m = _tone(x, j, harmonics)
        return m | {"sampled_gain_db": 20 * math.log10(m.pop("amplitude") / amplitude)}

    return DesignBench(
        name="sample_hold_dynamic",
        category="sample_hold",
        testbench=tb,
        intent="Spectrum of the held samples: SNDR, SFDR, THD and sampled gain.",
        measurements=["sndr_db", "snr_db", "sfdr_dbc", "thd_db", "hd2_db", "hd3_db", "enob_bits",
                      "sampled_gain_db"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["SNDR above target near Nyquist", "sampled gain within budget"],
        extractor=extract,
        runner=runner,
    )


def _sample_hold_ac(ps: Any, dut: Any, input_node: str, output_node: str, clock_node: str,
                    reference_node: str, hold_capacitance: float, clock_level: float,
                    input_dc: float) -> tuple[Any, Any]:
    """1 V AC into the input with the clock parked at ``clock_level``; C_H on the output."""
    tb = ps.Testbench(dut)
    tb.V(name="in", positive=input_node, negative=reference_node, value=input_dc, ac=1.0)
    tb.V(name="phi", positive=clock_node, negative=reference_node, value=clock_level)
    tb.C(name="hold", positive=output_node, negative=reference_node, value=hold_capacitance)
    tb.save(f"V({input_node})", f"V({output_node})")
    sweep = dict(variation="dec", number_of_points=50, start_frequency=1e3, stop_frequency=100e9)
    tb.add_ac(**sweep)
    return tb, lambda: tb.ac(**sweep)


def sample_hold_track_bandwidth(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    output_node: str = "vhold",
    clock_node: str = "phi",
    reference_node: str = "0",
    hold_capacitance: float = 1e-12,
    track_level: float = 1.0,
    input_dc: float = 0.5,
) -> DesignBench:
    """Small-signal track bandwidth and gain, clock held at the track level.

    A bootstrapped switch has no valid DC operating point in track (its
    capacitor charges only from a running clock), so this AC result is wrong
    for one; use ``sample_hold_dynamic``'s ``sampled_gain_db`` across fin there.
    ngspice-45's behavioural S switch is on in .op but stamps roff in .ac: model
    an ideal switch here as a B-source conductance.
    """
    tb, runner = _sample_hold_ac(ps, dut, input_node, output_node, clock_node, reference_node,
                                 hold_capacitance, track_level, input_dc)

    def extract(ac: Any) -> dict[str, float]:
        return {"track_bandwidth_hz": ac_bandwidth_hz(ac, output_node),
                "track_gain_db": ac_passband_db(ac, output_node)}

    return DesignBench(
        name="sample_hold_track_bandwidth",
        category="sample_hold",
        testbench=tb,
        intent="Track-mode small-signal bandwidth and gain into the hold capacitor.",
        measurements=["track_bandwidth_hz", "track_gain_db"],
        result_fields=["ac.frequency", f"ac.magnitude_db('{output_node}')"],
        validation=["track bandwidth above the settling requirement"],
        extractor=extract,
        runner=runner,
    )


def sample_hold_feedthrough(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    output_node: str = "vhold",
    clock_node: str = "phi",
    reference_node: str = "0",
    hold_capacitance: float = 1e-12,
    hold_level: float = 0.0,
    input_dc: float = 0.5,
    f_spec: float = 5e6,
) -> DesignBench:
    """Input-to-held-output feedthrough with the clock held at the hold level:
    ``feedthrough_db`` at ``f_spec`` (set it to fs/2) and the sweep maximum."""
    tb, runner = _sample_hold_ac(ps, dut, input_node, output_node, clock_node, reference_node,
                                 hold_capacitance, hold_level, input_dc)

    def extract(ac: Any) -> dict[str, float]:
        f = np.asarray(ac.frequency, float)
        db = np.asarray(ac.magnitude_db(output_node), float)
        return {"feedthrough_db": float(np.interp(math.log10(f_spec), np.log10(f), db)),
                "feedthrough_max_db": float(db.max())}

    return DesignBench(
        name="sample_hold_feedthrough",
        category="sample_hold",
        testbench=tb,
        intent="Hold-mode feedthrough from the input to the held output.",
        measurements=["feedthrough_db", "feedthrough_max_db"],
        result_fields=["ac.frequency", f"ac.magnitude_db('{output_node}')"],
        validation=["feedthrough below target at fs/2"],
        extractor=extract,
        runner=runner,
    )
