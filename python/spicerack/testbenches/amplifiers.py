"""Op-amp and amplifier benches: open-loop gain, loop gain, step response, noise.

Port, linearity and RF benches live in ``amplifiers_2``. A harness that needs
an element a ``Testbench`` cannot carry (L, E, G) is a wrapper ``Subcircuit``
around the DUT (``_harness``); everything else is plain ``Testbench`` stimulus.
"""

from __future__ import annotations

import json
import math
from typing import Any, Mapping, Sequence

import numpy as np

from .. import remeasure as rm  # module, not names: remeasure imports this package
from ._common import ac_complex, spot, tian
from .analysis import ac_bandwidth_hz, crossings
from .design import DesignBench, _step_pwl

#: Boltzmann constant, CODATA 2018 exact (J/K).
BOLTZMANN = 1.380649e-23
#: Peak-to-peak over rms for a Gaussian held 99.9% of the time (+-3.3 sigma),
#: the convention of ADI's 0.1-10 Hz noise specs. Some vendors use 6.
PP_PER_RMS = 6.6
#: DC servo: 1 GH from out to inn, 1 F from inn to AC ground. The L-C
#: resonance sits at 1/(2 pi sqrt(LC)) = 5.0e-6 Hz, so at 0.1 Hz the servo
#: leaks 1/(w^2 LC) = 2.5e-9 of v(out) back into inn.
SERVO_L, SERVO_C = 1e9, 1.0


def _ports(dut: Any) -> list[str]:
    """The DUT's port names in definition order."""
    return [p["name"] for p in json.loads(dut.to_json())["ports"]]


def _harness(ps: Any, dut: Any, name: str) -> Any:
    """A wrapper holding one DUT instance on nets named like its ports."""
    wrap = ps.Subcircuit(f"{name}_harness", [])
    wrap.instance(dut, "dut", *_ports(dut))
    return wrap


def _testbench(ps: Any, top: Any, dut: Any, bias: Mapping[str, float] | None) -> Any:
    """Testbench on ``top`` (the DUT or its harness), with a DC source per ``bias`` node."""
    tb = ps.Testbench(top)
    if top is not dut:
        tb.add_subcircuit(dut)
    for node, volts in (bias or {}).items():
        tb.V(name=f"bias_{node}", positive=node, negative="0", value=float(volts))
    return tb


def _load(tb: Any, node: str, capacitance: float | None, resistance: float | None) -> None:
    if capacitance:
        tb.C(name="load", positive=node, negative="0", value=capacitance)
    if resistance:
        tb.R(name="load", positive=node, negative="0", value=resistance)


def _mid(supplies: Mapping[str, float] | None, vcm: float | None) -> float:
    """``vcm``, else mid-rail of the supplies, else 0."""
    if vcm is not None:
        return float(vcm)
    rails = list((supplies or {}).values())
    return (max(rails) + min(rails)) / 2.0 if rails else 0.0


def _ac_args(start: float, stop: float, points_per_decade: int) -> dict[str, Any]:
    return dict(variation="dec", number_of_points=points_per_decade, start_frequency=start, stop_frequency=stop)


def _log_interp(f: np.ndarray, y: np.ndarray, at: float) -> float:
    """``y`` (positive) at ``at``, interpolated as a power law between sweep points."""
    if not f[0] <= at <= f[-1]:
        raise ValueError(f"{at:g} Hz lies outside the sweep [{f[0]:g}, {f[-1]:g}] Hz")
    return float(np.exp(np.interp(math.log(at), np.log(f), np.log(y))))


def _band_rms(f: np.ndarray, density: np.ndarray, lo: float, hi: float) -> float:
    """rms of ``density`` integrated over [lo, hi], ends interpolated as power laws."""
    inside = (f > lo) & (f < hi)
    fb = np.r_[lo, f[inside], hi]
    return rm._noise_total(fb, np.array([_log_interp(f, density, x) for x in fb]))


def opamp_open_loop(
    ps: Any,
    dut: Any,
    *,
    inp: str = "inp",
    inn: str = "inn",
    out: str = "out",
    supplies: Mapping[str, float] | None = None,
    vcm: float | None = None,
    load_capacitance: float | None = 1e-12,
    load_resistance: float | None = None,
    start_frequency: float = 0.1,
    stop_frequency: float = 1e10,
    points_per_decade: int = 50,
) -> DesignBench:
    """Open-loop A(jw) = v(out)/v(inp) at the follower's DC bias, through a DC servo.

    ``L(out, inn)`` closes a unity loop at DC only and ``C(inn, 0)`` grounds
    inn in AC, so the AC drive on inp sees the bare open-loop gain. The PM is
    for beta = 1 at this load (``pm_unity_deg``); a real feedback network
    loads the amp differently, so use ``loop_gain`` for the application PM.
    ``supplies`` maps DUT supply pins to volts; ``vcm`` defaults to mid-rail.
    """
    wrap = _harness(ps, dut, "open_loop")
    wrap.L(name="srv", positive=out, negative=inn, value=SERVO_L)
    wrap.C(name="srv", positive=inn, negative=wrap.gnd, value=SERVO_C)
    tb = _testbench(ps, wrap, dut, supplies)
    tb.V(name="cm", positive=inp, negative="0", value=_mid(supplies, vcm), ac=1.0)
    _load(tb, out, load_capacitance, load_resistance)
    ac_args = _ac_args(start_frequency, stop_frequency, points_per_decade)
    tb.add_operating_point()
    tb.add_ac(**ac_args)

    def extract(ac: Any) -> dict[str, float]:
        f, a = np.asarray(ac.frequency, float), ac_complex(ac, out)
        a0_db = 20.0 * math.log10(abs(a[0]))
        f_p1 = ac_bandwidth_hz(ac, out, reference_db=a0_db)
        if f_p1 < 10.0 * f[0]:
            raise ValueError(f"dominant pole {f_p1:g} Hz is within a decade of start_frequency={f[0]:g} Hz; "
                             "lower start_frequency so A(f_min) is the DC gain")
        m = rm._margins(f, a)
        return {
            "a0_db": a0_db,
            "f_p1_hz": f_p1,
            "gbw_hz": abs(a[0]) * f_p1,
            "ugf_hz": m["ugf_hz"],
            "pm_unity_deg": m["pm_deg"],
            "gm_unity_db": m["gm_db"],
        }

    return DesignBench(
        name="opamp_open_loop",
        category="opamp",
        testbench=tb,
        intent="Open-loop gain, dominant pole, GBW, UGF and unity-feedback margins at the follower bias.",
        measurements=["a0_db", "f_p1_hz", "gbw_hz", "ugf_hz", "pm_unity_deg", "gm_unity_db"],
        result_fields=["ac.frequency", f"ac complex v({out})"],
        validation=["a0 in budget", "ugf above target", "pm_unity above 60 deg"],
        extractor=extract,
        runner=lambda: tb.ac(**ac_args),
    )


def loop_gain(
    ps: Any,
    dut: Any,
    *,
    e: str = "e",
    f: str = "f",
    bias: Mapping[str, float] | None = None,
    start_frequency: float = 1.0,
    stop_frequency: float = 1e10,
    points_per_decade: int = 50,
) -> DesignBench:
    """Return ratio T(jw) of a closed loop broken at one wire, by Tian's two injections.

    The DUT is the whole closed loop (feedback network, load, bias generators)
    with the loop wire brought out as two ports: ``e`` the driven side (e.g.
    an op-amp's inn, an LDO error amp's feedback input) and ``f`` the driving
    side (the network tap). ``bias`` maps nodes to DC volts. T is exact for
    any impedance at the break and positive at DC for negative feedback
    (Tian et al., IEEE Circuits & Devices 2001). The wire must break every
    loop, or T is a partial return ratio. UGF and PM are read at the first
    |T| = 1 crossing (``remeasure.loop_margins``); a loop that crosses unity
    more than once, or a conditionally stable one, needs T itself and a
    transient. Two AC runs; the runner returns ``{"v": ac, "i": ac}`` so a
    corner loop can run the two decks itself.
    """
    ac_args = _ac_args(start_frequency, stop_frequency, points_per_decade)

    def build(v_ac: float, i_ac: float) -> Any:
        tb = _testbench(ps, dut, dut, bias)
        tb.V(name="prb", positive=e, negative=f, value=0.0, ac=v_ac)
        tb.I(name="prb", positive="0", negative=e, value=0.0, ac=i_ac)
        tb.add_operating_point()
        tb.add_ac(**ac_args)
        return tb

    def extract(runs: Mapping[str, Any]) -> dict[str, float]:
        v_run, i_run = runs["v"], runs["i"]
        t = tian(-ac_complex(i_run, "i(vprb)"), -ac_complex(v_run, "i(vprb)"),
                 ac_complex(i_run, e), ac_complex(v_run, e))
        m = rm._margins(np.asarray(v_run.frequency, float), t)
        return {"t0_db": 20.0 * math.log10(abs(t[0])), "ugf_hz": m["ugf_hz"], "pm_deg": m["pm_deg"],
                "gm_db": m["gm_db"], "phase_crossover_hz": m["pcf_hz"]}

    return DesignBench(
        name="loop_gain",
        category="stability",
        testbench=build(1.0, 0.0),
        intent="Loop gain T at a named break in the application loop: T0, UGF, PM, GM.",
        measurements=["t0_db", "ugf_hz", "pm_deg", "gm_db", "phase_crossover_hz"],
        result_fields=["ac.frequency", f"ac complex v({e}), i(Vprb) for each injection"],
        validation=["pm above 60 deg", "gm above 10 dB", "transient step agrees (conditional stability)"],
        extractor=extract,
        runner=lambda: {"v": build(1.0, 0.0).ac(**ac_args), "i": build(0.0, 1.0).ac(**ac_args)},
    )


def _tol_name(tol: float) -> str:
    return f"{tol * 100:g}pct".replace(".", "p")


def step_response(
    ps: Any,
    dut: Any,
    *,
    period: float,
    input_node: str = "vin",
    output_node: str = "vout",
    reference_node: str = "0",
    bias: Mapping[str, float] | None = None,
    v_low: float = 0.0,
    v_high: float = 1.0,
    ideal_gain: float = 1.0,
    tolerances: Sequence[float] = (1e-3, 1e-4),
    max_step: float | None = None,
    load_capacitance: float | None = None,
    load_resistance: float | None = None,
) -> DesignBench:
    """Closed-loop step up at ``period`` and back down at ``2*period``.

    ``period`` must hold each level for 10x the settling time. Edges last
    ``period/1000``, which must be fast against 1/UGF or the bench measures
    the source. ``max_step`` (default ``period/1e4``) bounds every time error:
    set it under tau/100 of the loop. Settling runs from the input's 50%
    point to the last exit from ``tol * |step|`` around the *measured* final
    value (mean of the last 5% of the level); a static gain error is reported
    apart, as ``static_error_pct`` against ``ideal_gain``. Slew is the 20-80%
    rate of each edge, which equals SR only when the step drives the input
    stage into slewing.
    """
    tb = _testbench(ps, dut, dut, bias)
    _step_pwl(tb, "in", input_node, reference_node, [v_low, v_high, v_low], period)
    _load(tb, output_node, load_capacitance, load_resistance)
    edge = period / 1000.0
    tran_args = dict(step_time=edge / 10.0, end_time=3.0 * period, max_time=max_step or period / 1e4)
    tb.add_transient(**tran_args)

    def extract(tran: Any) -> dict[str, float]:
        t, y = np.asarray(tran.time, float), np.asarray(tran[output_node], float)
        values: dict[str, float] = {}
        for name, k in (("rise", 1), ("fall", 2)):
            t_edge = k * period
            win = (t >= t_edge) & (t <= t_edge + period)
            tw, yw = t[win], y[win]
            v0 = float(np.interp(t_edge, t, y))
            final = float(np.mean(yw[tw >= t_edge + 0.95 * period]))
            step = final - v0
            for tol in tolerances:
                values[f"settle_{name}_{_tol_name(tol)}_s"] = rm._settling(tw, yw, final, tol, t_edge) - edge / 2.0
            values[f"overshoot_{name}_pct"] = max(float(np.max(np.sign(step) * (yw - final))), 0.0) / abs(step) * 100.0
            if k == 1:
                ideal = ideal_gain * (v_high - v_low)
                values["v_final"] = final
                values["static_error_pct"] = (step - ideal) / ideal * 100.0
        slew = rm._slew(t, y, 0.2, 0.8)
        values["slew_rise_v_per_s"], values["slew_fall_v_per_s"] = slew["rising"], slew["falling"]
        return values

    settles = [f"settle_{e}_{_tol_name(tol)}_s" for e in ("rise", "fall") for tol in tolerances]
    return DesignBench(
        name="step_response",
        category="opamp",
        testbench=tb,
        intent="Closed-loop step: settling at each tolerance, overshoot, slew rate, static error.",
        measurements=[*settles, "overshoot_rise_pct", "overshoot_fall_pct", "v_final", "static_error_pct",
                      "slew_rise_v_per_s", "slew_fall_v_per_s"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["settles within budget at each tolerance", "overshoot below target", "slew above target"],
        extractor=extract,
        runner=lambda: tb.transient(**tran_args),
    )


def noise_spectrum(
    ps: Any,
    dut: Any,
    *,
    band: tuple[float, float],
    input_node: str = "vin",
    output_node: str = "vout",
    reference_node: str = "0",
    input_bias: float = 0.0,
    source_resistance: float | None = None,
    bias: Mapping[str, float] | None = None,
    spots: Sequence[float] = (1e3,),
    start_frequency: float = 0.1,
    stop_frequency: float = 1e8,
    points_per_decade: int = 50,
    temperature: float = 27.0,
) -> DesignBench:
    """Output and input-referred noise of a biased block from one ``.noise`` run.

    The input source ``Vin`` (DC ``input_bias``, behind a noisy
    ``source_resistance`` when given) is the signal input; for a reference or
    regulator with no signal input, name its supply pin and read only the
    output columns. ``band`` is the application band and has no default.
    Integrated noise comes from the *output* density; ``in_rms_band`` divides
    it by |H| at the band's geometric centre, with |H| = onoise/inoise from
    the same run (integrating the input density diverges where |H| falls).
    ``out_vpp_0p1_10hz`` is ``PP_PER_RMS`` = 6.6 x rms(0.1-10 Hz).
    ``noise_equivalent_c_f`` is kT / v_rms^2 over the whole sweep: the
    capacitor whose kT/C noise equals this output (Noise/6b1 in AnalogTheory).
    """
    tb = _testbench(ps, dut, dut, bias)
    tb.temperature = temperature
    if source_resistance:
        tb.V(name="in", positive="vin_emf", negative=reference_node, value=input_bias, ac=1.0)
        tb.R(name="s", positive="vin_emf", negative=input_node, value=source_resistance)
    else:
        tb.V(name="in", positive=input_node, negative=reference_node, value=input_bias, ac=1.0)
    noise_args = dict(output_node=output_node, ref_node=reference_node, src="Vin", variation="dec",
                      points=points_per_decade, start_frequency=start_frequency, stop_frequency=stop_frequency)
    tb.add_operating_point()
    tb.add_noise(**noise_args)
    lo, hi = band

    def extract(nz: Any) -> dict[str, float]:
        f = np.asarray(nz.frequency, float)
        out_d, in_d = np.asarray(nz.onoise_spectrum, float), np.asarray(nz.inoise_spectrum, float)
        mid = math.sqrt(lo * hi)
        out_rms = _band_rms(f, out_d, lo, hi)
        total = rm._noise_total(f, out_d)
        values = {
            "out_rms_band_v": out_rms,
            "in_rms_band": out_rms * _log_interp(f, in_d, mid) / _log_interp(f, out_d, mid),
            "out_vpp_0p1_10hz": PP_PER_RMS * _band_rms(f, out_d, 0.1, 10.0),
            "out_rms_total_v": total,
            "noise_equivalent_c_f": BOLTZMANN * (temperature + 273.15) / total**2,
        }
        for x in spots:
            values[spot("out_density_at", x)] = _log_interp(f, out_d, x)
            values[spot("in_density_at", x)] = _log_interp(f, in_d, x)
        return values

    return DesignBench(
        name="noise_spectrum",
        category="noise",
        testbench=tb,
        intent="Output and input-referred spot noise, band rms, 0.1-10 Hz p-p and kT/C-equivalent capacitance.",
        measurements=["out_rms_band_v", "in_rms_band", "out_vpp_0p1_10hz", "out_rms_total_v", "noise_equivalent_c_f",
                      *(spot(p, x) for x in spots for p in ("out_density_at", "in_density_at"))],
        result_fields=["noise.frequency", "noise.onoise_spectrum", "noise.inoise_spectrum"],
        validation=["in-band output rms below budget", "spot density below datasheet"],
        extractor=extract,
        runner=lambda: tb.noise(**noise_args),
    )


def rf_compression_ip3(
    ps: Any,
    dut: Any,
    *,
    frequency: float,
    levels_dbm: Sequence[float],
    port1: str = "p1",
    port2: str = "p2",
    z0: float = 50.0,
    bias: Mapping[str, float] | None = None,
    tone_spacing: float | None = None,
    samples: int = 1024,
    steps_per_sample: int = 16,
) -> DesignBench:
    """Input 1 dB compression and two-tone IIP3 from coherent transients, one run per level.

    Levels are *available* source power per tone, P = Es^2 / (8 Z0) for an
    EMF of peak Es behind Z0 into ``port1``; ``port2`` sees a Z0 load and
    P_out = |V2|^2 / (2 Z0). Gain is P_out - P_in. A single tone runs at
    every level (``p1db_in_dbm`` interpolates where gain is 1 dB under the
    lowest level's; nan when the levels never compress that far). Two tones
    at f and f + ``tone_spacing`` (default f/100; f must be an integer
    multiple of it) run at the two lowest levels: ``iip3_dbm`` is
    P_in + (P_fund - P_im3)/2 at the lowest, valid only where ``im3_slope``
    is 3 dB/dB. The record is one beat period, so every tone and IM3
    product sits on its own bin.
    """
    df = tone_spacing or frequency / 100.0
    n1 = round(frequency / df)
    if abs(frequency / df - n1) > 1e-9 or 3 * (n1 + 2) >= samples // 2:
        raise ValueError(f"need frequency/tone_spacing an integer (got {frequency / df:.9g}) and "
                         f"samples > 6 (f/df + 2) = {6 * (n1 + 2)}")
    window = 1.0 / df
    fs = samples / window
    levels = sorted(float(x) for x in levels_dbm)
    if len(levels) < 2:
        raise ValueError("need at least two levels: IM3 slope and compression both compare levels")
    emf = lambda dbm: math.sqrt(8.0 * z0 * 1e-3 * 10.0 ** (dbm / 10.0))
    tran_args = dict(step_time=1.0 / fs, end_time=2.0 * window, max_time=1.0 / (steps_per_sample * fs))

    def build(dbm: float, tones: int) -> Any:
        tb = _testbench(ps, dut, dut, bias)
        tb.R(name="z1", positive="rf_s1", negative=port1, value=z0)
        tb.R(name="z2", positive=port2, negative="0", value=z0)
        tb.SinusoidalVoltageSource(name="p1", positive="rf_s1", negative="rf_m" if tones == 2 else "0",
                                   amplitude=emf(dbm), frequency=frequency)
        if tones == 2:
            tb.SinusoidalVoltageSource(name="p1b", positive="rf_m", negative="0", amplitude=emf(dbm),
                                       frequency=frequency + df)
        tb.add_transient(**tran_args)
        return tb

    def spectrum_dbm(tran: Any) -> np.ndarray:
        """Output power in dBm per bin of the last beat period (peak amplitude 2|X|/N)."""
        t, y = np.asarray(tran.time, float), np.asarray(tran[port2], float)
        amp = 2.0 * np.abs(np.fft.rfft(np.interp(window + np.arange(samples) / fs, t, y))) / samples
        return 10.0 * np.log10(np.maximum(amp, 1e-300) ** 2 / (2.0 * z0) / 1e-3)

    def extract(runs: Mapping[str, Any]) -> dict[str, float]:
        gain = np.array([spectrum_dbm(r)[n1] for r in runs["one"]]) - np.array(levels)
        hits = crossings(np.array(levels), gain, gain[0] - 1.0, rising=False)
        two = [spectrum_dbm(r) for r in runs["two"]]
        im3 = [max(s[n1 - 1], s[n1 + 2]) for s in two]
        iip3 = levels[0] + (two[0][n1] - im3[0]) / 2.0
        return {"gain_db": float(gain[0]), "p1db_in_dbm": hits[0] if hits else math.nan, "iip3_dbm": iip3,
                "oip3_dbm": iip3 + float(gain[0]), "im3_slope": (im3[1] - im3[0]) / (levels[1] - levels[0])}

    return DesignBench(
        name="rf_compression_ip3",
        category="rf",
        testbench=build(levels[0], 2),
        intent="Small-signal gain, input P1dB and two-tone IIP3/OIP3 at Z0, available-power convention.",
        measurements=["gain_db", "p1db_in_dbm", "iip3_dbm", "oip3_dbm", "im3_slope"],
        result_fields=["tran.time", f"tran['{port2}'] per level, one tone and two tones"],
        validation=["im3_slope within 3 +- 0.1", "p1db above the largest signal", "iip3 above target"],
        extractor=extract,
        runner=lambda: {"one": [build(x, 1).transient(**tran_args) for x in levels],
                        "two": [build(x, 2).transient(**tran_args) for x in levels[:2]]},
    )
