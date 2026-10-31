"""Amplifier benches, part 2: CMRR, offset/ICMR, port impedance, output swing,
harmonic distortion and RF two-port S-parameters with noise figure.

Shares the harness helpers of ``amplifiers``.
"""

from __future__ import annotations

import math
from typing import Any, Mapping, Sequence

import numpy as np

from .. import remeasure as rm  # module, not names: remeasure imports this package
from ._common import ac_complex, spot
from .amplifiers import BOLTZMANN, SERVO_C, SERVO_L, _ac_args, _harness, _load, _log_interp, _mid, _testbench
from .analysis import HALF_POWER_DB, crossings
from .design import DesignBench

#: IEEE reference temperature for noise figure (K).
T0_NOISE_FIGURE = 290.0


def _rails(supplies: Mapping[str, float] | None, sweep: tuple[float, float] | None) -> tuple[float, float]:
    if sweep is not None:
        return float(sweep[0]), float(sweep[1])
    if not supplies:
        raise ValueError("no supplies to span: pass sweep=(lo, hi)")
    return float(min(supplies.values())), float(max(supplies.values()))


def _edges(x: np.ndarray, y: np.ndarray, center: int, threshold: float, what: str) -> tuple[float, float]:
    """The outermost x around ``x[center]`` where ``y`` stays at or above ``threshold``,
    interpolated; raises when ``y`` never drops below it on one side."""
    if y[center] < threshold:
        raise ValueError(f"{what}: {y[center]:.6g} is already below {threshold:.6g} at {x[center]:.6g}")
    lo = crossings(x[:center + 1], y[:center + 1], threshold)
    hi = crossings(x[center:], y[center:], threshold)
    if not lo or not hi:
        raise ValueError(f"{what} never drops below {threshold:.6g} on one side of the sweep "
                         f"[{x[0]:.6g}, {x[-1]:.6g}]; widen sweep")
    return lo[-1], hi[0]


def _complex_at(f: np.ndarray, z: np.ndarray, at: float) -> complex:
    """``z`` at ``at``: |z| as a power law, phase (unwrapped) linear in ln f."""
    phase = np.interp(math.log(at), np.log(f), np.unwrap(np.angle(z)))
    return _log_interp(f, np.abs(z), at) * complex(math.cos(phase), math.sin(phase))


def cmrr(
    ps: Any,
    dut: Any,
    *,
    inp: str = "inp",
    inn: str = "inn",
    out: str = "out",
    supplies: Mapping[str, float] | None = None,
    vcm: float | None = None,
    servo: bool = True,
    spots: Sequence[float] = (1e3,),
    load_capacitance: float | None = None,
    load_resistance: float | None = None,
    start_frequency: float = 1.0,
    stop_frequency: float = 1e9,
    points_per_decade: int = 50,
) -> DesignBench:
    """CMRR(f) = |A_dm| / |A_cm| from two AC runs on one DUT (MC-safe).

    The inputs sit at ``cm`` (DC ``vcm``) plus +-vd/2: the DM run drives
    vd = 1 split around a still CM, the CM run drives cm = 1. ``servo=True``
    (open-loop op-amp) holds inn at the DC of out through the open-loop
    bench's L-C low-pass and adds the AC drive in series (``Vdn``), so no
    1 F capacitor couples two driven nodes: that coupling costs ngspice's
    AC solve the 1e-8 of vd a 80 dB CMRR needs. ``servo=False`` drives inn
    directly (in-amps, difference amps: the DUT carries its own feedback).
    ``cmrr_corner_hz`` is nan when CMRR stays within 3.0103 dB of its DC
    value across the sweep. The nominal CMRR of a symmetric DUT is
    mismatch-dominated: run it under Monte Carlo. The extractor raises on
    |A_cm| < 1e-12 |A_dm|.
    """
    ac_args = _ac_args(start_frequency, stop_frequency, points_per_decade)

    def build(mode: str) -> Any:
        wrap, neg = dut, "cmrr_cm"
        if servo:
            wrap, neg = _harness(ps, dut, "cmrr"), "cmrr_k"
            wrap.L(name="srv", positive=out, negative=neg, value=SERVO_L)
            wrap.C(name="srv", positive=neg, negative=wrap.gnd, value=SERVO_C)
        tb = _testbench(ps, wrap, dut, supplies)
        cm = 1.0 if mode == "cm" else 0.0
        half = 0.5 if mode == "dm" else 0.0
        tb.V(name="cm", positive="cmrr_cm", negative="0", value=_mid(supplies, vcm), ac=cm)
        tb.V(name="dp", positive=inp, negative="cmrr_cm", value=0.0, ac=half)
        # inn = neg + Vdn: the servo node carries no AC, so Vdn adds the CM drive itself.
        dn = (cm, 0.0) if servo and mode == "cm" else (half, 180.0)
        tb.V(name="dn", positive=inn, negative=neg, value=0.0, ac=dn[0], ac_phase=dn[1])
        _load(tb, out, load_capacitance, load_resistance)
        tb.add_operating_point()
        tb.add_ac(**ac_args)
        return tb

    def extract(runs: Mapping[str, Any]) -> dict[str, float]:
        f = np.asarray(runs["dm"].frequency, float)
        a_dm, a_cm = np.abs(ac_complex(runs["dm"], out)), np.abs(ac_complex(runs["cm"], out))
        if np.any(a_cm < 1e-12 * a_dm):
            raise ValueError("|A_cm| is below 1e-12 |A_dm|: a symmetric nominal DUT; run Monte Carlo")
        db = 20.0 * np.log10(a_dm / a_cm)
        corner = crossings(f, db, db[0] - HALF_POWER_DB, rising=False, log_x=True)
        values = {"cmrr_dc_db": float(db[0]), "cmrr_corner_hz": corner[0] if corner else math.nan,
                  "a_cm_db": 20.0 * math.log10(a_cm[0])}
        for x in spots:
            values[spot("cmrr_db_at", x)] = 20.0 * math.log10(_log_interp(f, a_dm / a_cm, x))
        return values

    return DesignBench(
        name="cmrr",
        category="opamp",
        testbench=build("dm"),
        intent="Common-mode rejection vs frequency: DC value, spot values, corner, raw CM gain.",
        measurements=["cmrr_dc_db", "cmrr_corner_hz", "a_cm_db", *(spot("cmrr_db_at", x) for x in spots)],
        result_fields=["ac.frequency", f"ac complex v({out}) for the dm and cm runs"],
        validation=["cmrr_dc above target (Monte Carlo mu - 3 sigma)", "corner above signal band"],
        extractor=extract,
        runner=lambda: {"dm": build("dm").ac(**ac_args), "cm": build("cm").ac(**ac_args)},
    )


def offset_icmr(
    ps: Any,
    dut: Any,
    *,
    inp: str = "inp",
    inn: str = "inn",
    out: str = "out",
    supplies: Mapping[str, float] | None = None,
    vcm: float | None = None,
    sweep: tuple[float, float] | None = None,
    step: float = 0.01,
    tol_gain: float = 0.01,
) -> DesignBench:
    """Offset, input bias currents and input common-mode range of a unity follower.

    The DUT's own loop: out drives inn through a 0 V ammeter ``Vamm``, inp is
    swept by ``Vin`` across ``sweep`` (default the supply span). ``vos_v`` is
    v(out) - v(inp) at ``vcm``, input-referred within 1/(1 + A0). Bias
    currents are into each pin. ICMR is the outermost input around ``vcm``
    where the incremental gain stays >= 1 - ``tol_gain``; a follower couples
    ICMR with output swing (out = in), so compare with ``output_swing``.
    Tolerances are tightened (reltol 1e-6, vntol 1 nV) for the derivative.
    """
    lo, hi = _rails(supplies, sweep)
    center = _mid(supplies, vcm)
    tb = _testbench(ps, dut, dut, supplies)
    tb.V(name="in", positive=inp, negative="0", value=center)
    tb.V(name="amm", positive=out, negative=inn, value=0.0)
    tb.options(reltol=1e-6, vntol=1e-9)
    sweep_args = {"Vin": slice(lo, hi, step)}
    tb.add_dc(**sweep_args)

    def extract(dc: Any) -> dict[str, float]:
        x, vo = np.asarray(dc.sweep, float), np.asarray(dc[out], float)
        at = lambda y: float(np.interp(center, x, np.asarray(y, float)))
        ib_pos, ib_neg = -at(dc["i(vin)"]), at(dc["i(vamm)"])
        icmr_lo, icmr_hi = _edges(x, np.gradient(vo, x), int(np.argmin(np.abs(x - center))), 1.0 - tol_gain,
                                  "follower gain")
        return {"vos_v": at(vo - x), "icmr_low_v": icmr_lo, "icmr_high_v": icmr_hi,
                "ib_pos_a": ib_pos, "ib_neg_a": ib_neg, "ios_a": ib_pos - ib_neg}

    return DesignBench(
        name="offset_icmr",
        category="opamp",
        testbench=tb,
        intent="Systematic offset, input bias/offset current and input common-mode range of a follower.",
        measurements=["vos_v", "icmr_low_v", "icmr_high_v", "ib_pos_a", "ib_neg_a", "ios_a"],
        result_fields=["dc.sweep", f"dc['{out}']", "dc['i(vin)']", "dc['i(vamm)']"],
        validation=["|vos| below budget (Monte Carlo for random offset)", "icmr covers the input range"],
        extractor=extract,
        runner=lambda: tb.dc(**sweep_args),
    )


def port_impedance(
    ps: Any,
    dut: Any,
    *,
    port: str = "out",
    drive: str = "current",
    reference_node: str = "0",
    port_bias: float = 0.0,
    bias: Mapping[str, float] | None = None,
    spots: Sequence[float] = (1e6,),
    start_frequency: float = 1.0,
    stop_frequency: float = 1e9,
    points_per_decade: int = 50,
) -> DesignBench:
    """Small-signal Z(f) = 1/Y(f) of one port at its bias point.

    ``drive="current"`` injects ``Itest`` (DC ``port_bias`` amps, AC 1 A) for
    ports that set their own voltage (amplifier and regulator outputs, TIA
    inputs); forcing those with a V source fights the loop. ``drive="voltage"``
    forces ``Vtest`` (DC ``port_bias`` volts, AC 1 V) for ports whose voltage
    the caller sets (gate inputs, current outputs at a compliance voltage).
    ``r_parallel_ohm`` is 1/Re(Y) at the lowest frequency; ``c_parallel_f`` is
    Im(Y)/w at the first spot, the shunt C of an R || C reading (pick a spot
    where wC >> 1/R and below any series-R corner). ``z_peak_*`` refines the
    largest |Z| sample by a parabola in ln f: closed-loop output peaking.
    """
    if drive not in ("current", "voltage"):
        raise ValueError(f"drive must be 'current' or 'voltage', not {drive!r}")
    tb = _testbench(ps, dut, dut, bias)
    if drive == "current":
        tb.I(name="test", positive=reference_node, negative=port, value=port_bias, ac=1.0)
    else:
        tb.V(name="test", positive=port, negative=reference_node, value=port_bias, ac=1.0)
    ac_args = _ac_args(start_frequency, stop_frequency, points_per_decade)
    tb.add_operating_point()
    tb.add_ac(**ac_args)

    def extract(ac: Any) -> dict[str, float]:
        f = np.asarray(ac.frequency, float)
        z = ac_complex(ac, port) if drive == "current" else -1.0 / ac_complex(ac, "i(vtest)")
        mag = np.abs(z)
        k = int(np.argmax(mag))
        peak_lf, peak_lm = math.log(f[k]), math.log(mag[k])
        if 0 < k < len(f) - 1:  # an interior maximum: the parabola's vertex lies inside its bracket
            a, b, c = np.polyfit(np.log(f[k - 1:k + 2]), np.log(mag[k - 1:k + 2]), 2)
            if a < 0:
                peak_lf, peak_lm = -b / (2 * a), c - b * b / (4 * a)
        values = {
            "z_dc_ohm": float(mag[0]),
            "r_parallel_ohm": float(1.0 / (1.0 / z[0]).real),
            "c_parallel_f": float((1.0 / _complex_at(f, z, spots[0])).imag / (2 * math.pi * spots[0])),
            "z_peak_ohm": float(math.exp(peak_lm)),
            "z_peak_hz": float(math.exp(peak_lf)),
        }
        for x in spots:
            zx = _complex_at(f, z, x)
            values[spot("z_ohm_at", x)] = abs(zx)
            values[spot("z_phase_deg_at", x)] = math.degrees(np.angle(zx))
        return values

    return DesignBench(
        name="port_impedance",
        category="port",
        testbench=tb,
        intent="Port impedance and admittance vs frequency: DC R, parallel C, spot |Z| and phase, peaking.",
        measurements=["z_dc_ohm", "r_parallel_ohm", "c_parallel_f", "z_peak_ohm", "z_peak_hz",
                      *(spot(p, x) for x in spots for p in ("z_ohm_at", "z_phase_deg_at"))],
        result_fields=["ac.frequency", f"ac complex v({port}) or i(Vtest)"],
        validation=["output impedance below budget", "no peaking above target (PM symptom)"],
        extractor=extract,
        runner=lambda: tb.ac(**ac_args),
    )


def output_swing(
    ps: Any,
    dut: Any,
    *,
    inp: str = "inp",
    inn: str = "inn",
    out: str = "out",
    supplies: Mapping[str, float] | None = None,
    vref: float | None = None,
    resistance: float = 100e3,
    sweep: tuple[float, float] | None = None,
    step: float = 1e-3,
    drop: float = 0.10,
) -> DesignBench:
    """Usable output range: where the open-loop gain falls ``drop`` below its peak.

    Inverting gain of -1 (``resistance`` for both Rg and Rf, a load the result
    is stated at) around ``vref`` on inp (default mid-rail); ``Vin`` sweeps
    ``sweep`` (default the supply span). A_ol(Vout) = dVout / d(v(inp) -
    v(inn)) by central differences, with reltol 1e-7 and vntol 0.1 nV because
    v(inp) - v(inn) is Vout/A0. This DC swing is not the swing at a THD limit.
    """
    lo, hi = _rails(supplies, sweep)
    tb = _testbench(ps, dut, dut, supplies)
    tb.V(name="ref", positive=inp, negative="0", value=_mid(supplies, vref))
    tb.V(name="in", positive="swing_in", negative="0", value=_mid(supplies, vref))
    tb.R(name="g", positive="swing_in", negative=inn, value=resistance)
    tb.R(name="f", positive=inn, negative=out, value=resistance)
    tb.options(reltol=1e-7, vntol=1e-10)
    sweep_args = {"Vin": slice(lo, hi, step)}
    tb.add_dc(**sweep_args)

    def extract(dc: Any) -> dict[str, float]:
        x, vo = np.asarray(dc.sweep, float), np.asarray(dc[out], float)
        vd = np.asarray(dc[inp], float) - np.asarray(dc[inn], float)
        a = np.gradient(vo, x) / np.gradient(vd, x)
        order = np.argsort(vo)
        vo, a = vo[order], a[order]
        peak = int(np.argmax(a))
        v_min, v_max = _edges(vo, a, peak, (1.0 - drop) * a[peak], "open-loop gain")
        return {"vout_min_v": v_min, "vout_max_v": v_max, "swing_pp_v": v_max - v_min,
                "a_ol_peak_db": 20.0 * math.log10(a[peak])}

    return DesignBench(
        name="output_swing",
        category="opamp",
        testbench=tb,
        intent="Output range where the open-loop gain stays within a stated drop of its peak.",
        measurements=["vout_min_v", "vout_max_v", "swing_pp_v", "a_ol_peak_db"],
        result_fields=["dc.sweep", f"dc['{out}']", f"dc['{inp}'] - dc['{inn}']"],
        validation=["swing covers the signal range at the stated load"],
        extractor=extract,
        runner=lambda: tb.dc(**sweep_args),
    )


def harmonic_distortion(
    ps: Any,
    dut: Any,
    *,
    input_node: str = "vin",
    output_node: str = "vout",
    reference_node: str = "0",
    bias: Mapping[str, float] | None = None,
    frequency: float = 1e3,
    amplitude: float = 0.1,
    offset: float = 0.0,
    cycles: int = 5,
    settle_cycles: int = 5,
    samples: int = 1024,
    max_harmonic: int = 9,
    steps_per_sample: int = 16,
) -> DesignBench:
    """Single-tone THD, HD2, HD3, SFDR and SINAD from a coherent transient record.

    The record is the last ``cycles`` periods after ``settle_cycles``, sampled
    at fs = samples * frequency / cycles; ``cycles`` must be prime to
    ``samples`` (odd for a power of two) so the tone owns one bin and a
    rectangular window leaks nothing. Samples are interpolated between
    simulator points at most 1/(``steps_per_sample`` fs) apart, which bounds
    the floor near -125 dBc. Settle past the loop's start-up and any slew:
    check amplitude * 2 pi frequency < SR, or this measures slew distortion.
    """
    fs = samples * frequency / cycles
    tb = _testbench(ps, dut, dut, bias)
    tb.SinusoidalVoltageSource(name="in", positive=input_node, negative=reference_node, dc_offset=offset,
                               offset=offset, amplitude=amplitude, frequency=frequency)
    t0 = settle_cycles / frequency
    tran_args = dict(step_time=1.0 / fs, end_time=t0 + cycles / frequency, max_time=1.0 / (steps_per_sample * fs))
    tb.add_transient(**tran_args)

    def extract(tran: Any) -> dict[str, float]:
        t, y = np.asarray(tran.time, float), np.asarray(tran[output_node], float)
        r = rm._sndr(t, y, fs, frequency, samples, None, t0, max_harmonic)
        bins = np.abs(np.fft.rfft(np.interp(t0 + np.arange(samples) / fs, t, y)))
        hd = lambda k: 20.0 * math.log10(bins[k * cycles] / bins[cycles])
        return {"thd_db": r["thd_db"], "thd_pct": 100.0 * 10.0 ** (r["thd_db"] / 20.0), "hd2_dbc": hd(2),
                "hd3_dbc": hd(3), "sfdr_dbc": r["sfdr_db"], "sndr_db": r["sndr_db"]}

    return DesignBench(
        name="harmonic_distortion",
        category="linearity",
        testbench=tb,
        intent="Single-tone harmonic distortion of a closed-loop amplifier at a stated amplitude and frequency.",
        measurements=["thd_db", "thd_pct", "hd2_dbc", "hd3_dbc", "sfdr_dbc", "sndr_db"],
        result_fields=["tran.time", f"tran['{output_node}']"],
        validation=["thd below budget at full-scale amplitude", "amplitude below the slew limit"],
        extractor=extract,
        runner=lambda: tb.transient(**tran_args),
    )


def rf_two_port(
    ps: Any,
    dut: Any,
    *,
    frequency: float,
    port1: str = "p1",
    port2: str = "p2",
    z0: float = 50.0,
    bias: Mapping[str, float] | None = None,
    start_frequency: float = 1e6,
    stop_frequency: float = 1e10,
    points_per_decade: int = 50,
    temperature: float = 27.0,
) -> DesignBench:
    """S-parameters and noise figure of a two-port at ``frequency``, by plain AC.

    Each port sees an EMF behind Z0: ``Vp1`` behind a (noisy) resistor ``Rz1``,
    ``Vp2`` behind a noiseless G-resistor in the harness. A 2 V EMF makes the
    incident wave 1 V, so S11 = v(p1) - 1 and S21 = v(p2) with port 1
    driven (``fwd``), and the mirror with port 2 driven (``rev``). NF comes
    from ``.noise v(p2)`` referred to ``Vp1``: F = inoise^2 / (4 k T Z0) at
    the simulation temperature, reported at T0 = 290 K as
    1 + (F - 1) T / 290 (the load is noiseless, as the IEEE definition wants).
    Equal Z0 at both ports. Small-signal at the OP: no compression here.
    """
    ac_args = _ac_args(start_frequency, stop_frequency, points_per_decade)

    def build(e1: float, e2: float) -> Any:
        wrap = _harness(ps, dut, "rf")
        wrap.G(name="z2", positive=port2, negative="rf_s2", control_positive=port2, control_negative="rf_s2",
               transconductance=1.0 / z0)
        tb = _testbench(ps, wrap, dut, bias)
        tb.temperature = temperature
        tb.V(name="p1", positive="rf_s1", negative="0", value=0.0, ac=e1)
        tb.R(name="z1", positive="rf_s1", negative=port1, value=z0)
        tb.V(name="p2", positive="rf_s2", negative="0", value=0.0, ac=e2)
        tb.add_operating_point()
        tb.add_ac(**ac_args)
        return tb

    noise_args = dict(output_node=port2, ref_node="0", src="Vp1", variation="dec", points=points_per_decade,
                      start_frequency=start_frequency, stop_frequency=stop_frequency)

    def extract(runs: Mapping[str, Any]) -> dict[str, float]:
        f = np.asarray(runs["fwd"].frequency, float)
        # Real and imaginary parts linear in ln f: S11 of a matched port passes near zero.
        at = lambda run, node, inc: complex(*(np.interp(math.log(frequency), np.log(f), part(ac_complex(
            runs[run], node) - inc)) for part in (np.real, np.imag)))
        s = {"s11": at("fwd", port1, 1.0), "s21": at("fwd", port2, 0.0),
             "s12": at("rev", port1, 0.0), "s22": at("rev", port2, 1.0)}
        nz = runs["noise"]
        t_sim = temperature + 273.15
        density = _log_interp(np.asarray(nz.frequency, float), np.asarray(nz.inoise_spectrum, float), frequency)
        f_sim = density**2 / (4.0 * BOLTZMANN * t_sim * z0)
        return {**{f"{k}_db": 20.0 * math.log10(abs(v)) for k, v in s.items()},
                "nf_db": 10.0 * math.log10(1.0 + (f_sim - 1.0) * t_sim / T0_NOISE_FIGURE)}

    def run() -> dict[str, Any]:
        return {"fwd": build(2.0, 0.0).ac(**ac_args), "rev": build(0.0, 2.0).ac(**ac_args),
                "noise": build(1.0, 0.0).noise(**noise_args)}

    return DesignBench(
        name="rf_two_port",
        category="rf",
        testbench=build(2.0, 0.0),
        intent="S11, S21, S12, S22 and noise figure at Z0 without .sp (2 V EMF behind Z0).",
        measurements=["s11_db", "s21_db", "s12_db", "s22_db", "nf_db"],
        result_fields=["ac.frequency", f"ac complex v({port1}), v({port2}) per driven port", "noise.inoise_spectrum"],
        validation=["s11 below -10 dB in band", "s21 above target gain", "nf below target"],
        extractor=extract,
        runner=run,
    )
