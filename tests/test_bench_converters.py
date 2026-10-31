"""Converter benches against behavioural DUTs whose answer is known in closed form.

Every expected value is computed here, independently of the bench's extractor:
a closed form, or a numpy model of the ideal circuit read through a 3-parameter
sine fit. Each tolerance carries the line that derives it.
"""
import math

import numpy as np
import pytest

import spicerack as ps
from spicerack.testbenches.converters import (
    adc_dynamic,
    adc_static_linearity,
    coherent_bin,
    comparator_delay,
    comparator_threshold,
)
from spicerack.testbenches.converters_3 import (
    comparator_kickback,
    dac_output_impedance,
    pipeline_stage_residue,
)
from spicerack.testbenches.converters_2 import (
    dac_dynamic,
    dac_transition,
    sample_hold_dynamic,
    sample_hold_feedthrough,
    sample_hold_track_bandwidth,
)

SW = ".model swq SW(ron=1 roff=1e12 vt=0.5 vh=0)"
# Every relative approx below also passes abs=0: pytest's default abs=1e-12
# would swallow a factor-of-two error in a 25 fC charge or a 1e-10 V*s glitch.
#: RC DAC timing at max step tau/100: measured <= 3e-5 relative from the closed
#: forms, falling 20x at tau/400, so 1e-4.
TOL_T = 1e-4
#: A smooth minimum read on solver points: 6.4e-4 at tau/100 on the skewed dip, so 2e-3.
TOL_PEAK = 2e-3
#: RC sampler gain: 1.4e-6 dB measured (incomplete tracking e^-25 adds 1e-10 dB).
TOL_SH_DB = 1e-4
#: Numerical floor of the linear sampler at reltol 1e-5, measured 87 dB.
SH_FLOOR_DB = 80.0


def _sinad_by_fit(x, j):
    """SINAD of a coherent record by 3-parameter least squares: no FFT."""
    ph = 2 * np.pi * j * np.arange(len(x)) / len(x)
    basis = np.column_stack([np.sin(ph), np.cos(ph), np.ones(len(x))])
    coef, *_ = np.linalg.lstsq(basis, x, rcond=None)
    residual = x - basis @ coef
    return 10 * np.log10((coef[0] ** 2 + coef[1] ** 2) / 2 / np.mean(residual ** 2)), math.hypot(coef[0], coef[1])


def _quantizer(name, bits, fs=1.0, transfer="V(vin)", vh=1.0, ports=("vin",)):
    """Track while clk is high onto 1 pF through 1 ohm, quantize the held node.

    ``transfer`` shapes the input ahead of the sampler. Bit k is
    floor(x/lsb/2^k) mod 2 of the held x clamped to [0, fs - lsb/2]: ngspice
    parses 0.984375 a hair low, so clamping at fs - lsb floors to the code below.
    """
    lsb = fs / 2 ** bits
    outs = [f"d{i}" for i in range(bits)]
    dut = ps.Subcircuit(name, [*ports, "clk"] + outs)
    dut.raw_spice(SW)
    dut.BV(name="pre", positive="pre", negative="0", expression=transfer)
    dut.S(name="trk", positive="pre", negative="h", control_positive="clk", control_negative="0", model="swq")
    dut.C(name="h", positive="h", negative="0", value=1e-12)
    x = f"min(max(V(h),0),{fs - lsb / 2})/{lsb}"
    for i, node in enumerate(outs):
        dut.BV(name=f"b{i}", positive=node, negative="0",
               expression=f"{vh}*(floor({x}/{2 ** i})-2*floor({x}/{2 ** (i + 1)}))")
    return dut, outs


# ── coherent_bin ──

def test_coherent_bin_is_a_prime_with_distinct_folded_harmonics():
    # Spec examples: the nearest prime to 0.1 * 1024 = 102.4 is 103; near Nyquist 0.49 * 1024 -> 503.
    assert coherent_bin(1024, 0.1) == 103
    assert coherent_bin(1024, 0.49) == 503
    # M = 8: J = 3 folds H3 onto bin 1 and H5 onto 1 as well (3+5 = 8), so no prime below 4 works.
    with pytest.raises(ValueError):
        coherent_bin(8, 0.4, harmonics=5)
    with pytest.raises(ValueError):
        coherent_bin(1000, 0.1)


def test_spectrum_core_agrees_with_remeasure_sndr():
    """Differential oracle: remeasure._sndr is a separate implementation of the same definitions."""
    from spicerack.remeasure import _sndr
    from spicerack.testbenches.converters import _tone

    n, j = 1024, coherent_bin(1024, 0.1)
    k = np.arange(n)
    x = np.sin(2 * np.pi * j * k / n) + 1e-3 * np.sin(2 * np.pi * 3 * j * k / n) \
        + 1e-4 * np.random.default_rng(7).standard_normal(n)
    ours, theirs = _tone(x, j, harmonics=6), _sndr(k.astype(float), x, 1.0, j / n, n, t0=0.0)
    assert ours["sndr_db"] == pytest.approx(theirs["sndr_db"], abs=1e-9)
    assert ours["sfdr_dbc"] == pytest.approx(theirs["sfdr_db"], abs=1e-9)
    assert ours["thd_db"] == pytest.approx(theirs["thd_db"], abs=1e-9)
    assert ours["enob_bits"] == pytest.approx(theirs["enob"], abs=1e-9)
    # Noise in one bin has amplitude ~ 1e-4 * sqrt(2/n) = 4.4e-6 against 1e-3:
    # 0.04 dB at 1 sigma, so 0.15 dB.
    assert ours["hd3_db"] == pytest.approx(20 * math.log10(1e-3), abs=0.15)


# ── adc_dynamic ──

def test_adc_dynamic_ideal_quantizer_matches_model_and_closed_form():
    bits, record, period, a_dbfs = 6, 256, 1e-6, -0.5
    dut, outs = _quantizer("adcq", bits)
    bench = adc_dynamic(ps, dut, output_nodes=outs, record=record, conversion_period=period,
                        amplitude_dbfs=a_dbfs, settle_conversions=4)
    m = bench.measure()

    # Model: the ideal quantizer applied in numpy to the source where the switch
    # opens, the falling clock's 0.5 V crossing at rise + width + fall/2 = (0.5 + 1.5e-3) T.
    j = coherent_bin(record, 0.1)
    amp = 0.5 * 10 ** (a_dbfs / 20)
    t = (4 + np.arange(record) + 0.5 + 1.5e-3) * period
    codes = np.floor(np.clip(0.5 + amp * np.sin(2 * np.pi * j * t / (record * period)), 0, 1 - 1 / 64) * 64)
    sinad, a_fit = _sinad_by_fit(codes, j)
    # The simulated codes equal the model's bit for bit (held node, 1 ps track
    # error against a 15.6 mV LSB), so only float rounding separates the two.
    assert m["sndr_db"] == pytest.approx(sinad, abs=1e-9)
    assert m["signal_dbfs"] == pytest.approx(20 * math.log10(a_fit / 32), abs=1e-9)
    # Closed form 6.02N + 1.76 + A_dBFS; the spec measured at most 0.19 dB of
    # record-dependent deviation at N = 6, so +-0.25 dB.
    assert m["sndr_db"] == pytest.approx(6.0206 * bits + 1.7609 + a_dbfs, abs=0.25)
    # ENOB_fs removes the drive level: N +- 0.25 dB / 6.02 dB/bit.
    assert m["enob_fs_bits"] == pytest.approx(bits, abs=0.25 / 6.02)
    assert m["input_frequency_hz"] == pytest.approx(j / (record * period), rel=1e-12, abs=0)
    assert m["outlier_count"] == 0.0

    # Differential drive: two antiphase half-swing sines, recombined by the DUT, give the same codes.
    diff, _ = _quantizer("adcd", bits, transfer="0.5+V(vin)-V(vinn)", ports=("vin", "vinn"))
    bench = adc_dynamic(ps, diff, output_nodes=outs, input_node_n="vinn", record=record,
                        conversion_period=period, amplitude_dbfs=a_dbfs, settle_conversions=4)
    assert bench.measure()["sndr_db"] == pytest.approx(sinad, abs=1e-9)


def test_adc_dynamic_cubic_front_end_hd3_matches_closed_form():
    bits, record, a3 = 16, 256, 0.032
    amp = 0.5 * 10 ** (-0.5 / 20)
    # Explicit product: ngspice's ^ on a negative base is not the signed cube.
    u = "(V(vin)-0.5)"
    dut, outs = _quantizer("adcc", bits, transfer=f"0.5+{u}+{a3}*{u}*{u}*{u}")
    bench = adc_dynamic(ps, dut, output_nodes=outs, record=record, settle_conversions=4)
    m = bench.measure()
    # y = x + a3 x^3 on A sin: fundamental A + 3 a3 A^3 / 4, third harmonic a3 A^3 / 4.
    hd3 = 20 * math.log10((a3 * amp ** 3 / 4) / (amp + 3 * a3 * amp ** 3 / 4))
    assert -65 < hd3 < -55, "keep HD3 far above the 16-bit quantization spurs (~-130 dBc)"
    # 16-bit quantization noise in the HD3 bin sits ~64 dB under it, 0.005 dB of
    # amplitude; measured 0.004 dB at the bench's reltol 1e-5 (0.17 dB at 1e-3).
    assert m["hd3_db"] == pytest.approx(hd3, abs=0.02)
    assert m["thd_db"] == pytest.approx(hd3, abs=0.02)  # a cubic makes no other harmonic
    assert m["sfdr_dbc"] == pytest.approx(-hd3, abs=0.02)
    assert m["hd2_db"] < hd3 - 40  # odd transfer: HD2 is quantization only


# ── adc_static_linearity ──

def _endpoint(t):
    """DNL, INL (end-point fit) and LSB of transition levels t[k-1] = T[k], in LSB units."""
    lsb = (t[-1] - t[0]) / (len(t) - 1)
    return np.diff(t) / lsb - 1, (t - (t[0] + np.arange(len(t)) * lsb)) / lsb, lsb


def test_adc_static_linearity_ramp_recovers_a_widened_code_gain_and_offset():
    bits, g, o = 6, 1.02, 0.3  # o in LSB
    lsb = 1 / 2 ** bits
    dut, outs = _quantizer("adcw", bits, transfer=f"{g}*V(vin)+{o * lsb}-{lsb / 2}*u(V(vin)-0.5)")
    bench = adc_static_linearity(ps, dut, output_nodes=outs)
    m = bench.measure()
    # Transitions in LSB: y = g x + o crosses k below x = 32 LSB; above, the
    # -0.5 LSB jump moves every later crossing to (k + 0.5 - o) / g.
    k = np.arange(1, 2 ** bits, dtype=float)
    t = (k - o) / g
    t = np.where(t < 2 ** (bits - 1), t, (k + 0.5 - o) / g)
    dnl, inl, ep = _endpoint(t)
    assert dnl.max() > 0.45, "the defect must be visible, not a degenerate zero"
    # Each transition is placed to half a ramp step, 0.5/16 LSB; DNL takes two: 1/16.
    assert m["dnl_max_lsb"] == pytest.approx(dnl.max(), abs=1 / 16)
    assert m["dnl_min_lsb"] == pytest.approx(dnl.min(), abs=1 / 16)
    assert m["inl_max_lsb"] == pytest.approx(np.abs(inl).max(), abs=1 / 16)
    assert m["offset_error_lsb"] == pytest.approx(t[0] - 1, abs=0.5 / 16)
    # Gain spreads the 1/16 LSB end-point error over 62 codes.
    assert m["gain_error"] == pytest.approx(ep - 1, abs=1 / 16 / 62)
    assert m["missing_codes"] == 0.0


def test_adc_static_linearity_ramp_counts_a_missing_code():
    bits = 6
    lsb = 1 / 2 ** bits
    dut, outs = _quantizer("adcm", bits, transfer=f"V(vin)+{lsb}*u(V(vin)-0.5)")
    m = adc_static_linearity(ps, dut, output_nodes=outs).measure()
    # A +1 LSB jump at mid-scale skips code 32: its two transitions coincide.
    assert m["missing_codes"] == 1.0
    assert m["dnl_min_lsb"] == pytest.approx(-1.0, abs=1e-9)


def test_adc_static_linearity_sine_histogram_of_gain_and_offset():
    bits, g, o = 6, 0.98, 0.25
    lsb = 1 / 2 ** bits
    dut, outs = _quantizer("adcs", bits, transfer=f"{g}*V(vin)+{o * lsb}")
    m = adc_static_linearity(ps, dut, output_nodes=outs, stimulus="sine", samples_per_code=64).measure()
    # Linear transfer: DNL = INL = 0; T[1] = (1 - o)/g LSB, LSB_ep = 1/g.
    # Tolerances: 1.4x the worst floor of the arcsine estimate over 50 random
    # phases in numpy at N = 6, 4096 samples (DNL 0.037, INL 0.025, offset
    # 0.0074, gain 2.4e-4).
    assert abs(m["dnl_max_lsb"]) < 0.05 and abs(m["dnl_min_lsb"]) < 0.05
    assert m["inl_max_lsb"] < 0.035
    assert m["offset_error_lsb"] == pytest.approx((1 - o) / g - 1, abs=0.01)
    assert m["gain_error"] == pytest.approx(1 / g - 1, abs=3.5e-4)
    assert m["missing_codes"] == 0.0


# ── comparators ──

def _hysteretic_comparator(vos, vh):
    """Track d = Vd - Vos +- Vh/2 (sign of the last decision) while clk is low,
    hold it on 10 fF while high; out = u(held). Thresholds: Vos +- Vh/2."""
    dut = ps.Subcircuit("cmp", ["inp", "inn", "clk", "outp"])
    dut.raw_spice(".model swc SW(ron=1k roff=1e12 vt=0.5 vh=0)")
    dut.BV(name="d", positive="d", negative="0",
           expression=f"V(inp)-V(inn)-{vos}+{vh / 2}*(2*V(outp)-1)")
    dut.BV(name="clkb", positive="clkb", negative="0", expression="1-V(clk)")
    dut.S(name="trk", positive="d", negative="h", control_positive="clkb", control_negative="0", model="swc")
    dut.C(name="h", positive="h", negative="0", value=10e-15)
    # 10 ps of RC after the slicer keeps the feedback loop from being algebraic.
    dut.BV(name="sl", positive="sl", negative="0", expression="u(V(h))")
    dut.R(name="o", positive="sl", negative="outp", value=1e3)
    dut.C(name="o", positive="outp", negative="0", value=10e-15)
    return dut


def test_comparator_threshold_recovers_offset_and_hysteresis():
    vos, vh, span, steps = 3.33e-3, 1.04e-3, 20e-3, 400
    bench = comparator_threshold(ps, _hysteretic_comparator(vos, vh), span=span, steps=steps)
    m = bench.measure()
    step = 2 * span / steps
    # Up at Vos + Vh/2 = 3.85 mV, down at 2.81 mV, off the 0.1 mV grid. The bench
    # reports the midpoint of the two grid levels that bracket each threshold.
    bracket = lambda v: -span + step * (math.floor((v + span) / step) + 0.5)
    up, down = bracket(vos + vh / 2), bracket(vos - vh / 2)
    assert m["threshold_up_v"] == pytest.approx(up, abs=1e-12)
    assert m["threshold_down_v"] == pytest.approx(down, abs=1e-12)
    assert m["offset_v"] == pytest.approx((up + down) / 2, abs=1e-12)
    assert m["hysteresis_v"] == pytest.approx(up - down, abs=1e-12)
    assert abs(m["offset_v"] - vos) <= step / 2 and abs(m["hysteresis_v"] - vh) <= step
    assert m["chatter_count"] == 0.0
    assert m["resolution_v"] == pytest.approx(step)


def _tanh_latch(a0, tau, vl, cx=1e-12, vcm=0.5):
    """Reset (clk low): x = a0 (inp - inn). Evaluate (clk high): C dx/dt =
    C (vl/tau) tanh(x/vl), so sinh(x/vl) = sinh(x0/vl) e^(t/tau)."""
    dut = ps.Subcircuit("latch", ["inp", "inn", "clk", "outp", "outn"])
    # The switch opens at clk = 0.45, before the regeneration current starts at
    # 0.5: overlapping them adds I*ron = 10 % to a 5 uV x0 for one time step.
    dut.raw_spice(".model swl SW(ron=1 roff=1e12 vt=0.55 vh=0)")
    dut.E(name="a", positive="e", negative="0", control_positive="inp", control_negative="inn", voltage_gain=a0)
    dut.BV(name="clkb", positive="clkb", negative="0", expression="1-V(clk)")
    dut.S(name="rst", positive="e", negative="x", control_positive="clkb", control_negative="0", model="swl")
    dut.C(name="x", positive="x", negative="0", value=cx)
    dut.BI(name="regen", positive="0", negative="x",
           expression=f"{cx * vl / tau}*tanh(V(x)/{vl})*u(V(clk)-0.5)")
    dut.BV(name="p", positive="outp", negative="0", expression=f"{vcm}+V(x)/2")
    dut.BV(name="n", positive="outn", negative="0", expression=f"{vcm}-V(x)/2")
    return dut


def test_comparator_delay_matches_tanh_latch_closed_form():
    a0, tau, vl, vth = 5.0, 10e-12, 0.5, 0.2
    ods = (1e-2, 1e-3, 1e-4, 1e-5, 1e-6)
    bench = comparator_delay(ps, _tanh_latch(a0, tau, vl), outn_node="outn", overdrives=ods,
                             decide_level=vth, max_step=tau / 200)
    m = bench.measure()
    td = lambda od: tau * math.log(math.sinh(vth / vl) / math.sinh(a0 * od / vl))
    # Step halving (tau/50, /200, /1000) put the delays 1.9e-3, 6e-4, 4e-7 tau
    # from the closed form: tau/200 holds 2e-3 tau with 3x margin.
    assert m["delay_max_od_s"] == pytest.approx(td(1e-2), abs=2e-3 * tau)
    assert m["delay_min_od_s"] == pytest.approx(td(1e-6), abs=2e-3 * tau)
    # Slope -tau * a coth(a), a = A0 Vod / Vl <= 0.01: tau to 3.3e-5; measured
    # 1.2e-4 at tau/200, so 5e-4.
    assert m["tau_regen_s"] == pytest.approx(tau, rel=5e-4, abs=0)
    # v_meta carries the tau error times (T/2 - t_d)/tau ~ 39: 2 %.
    assert m["v_meta_v"] == pytest.approx(1e-6 * math.exp(-(0.5e-9 - td(1e-6)) / tau), rel=0.02, abs=0)
    assert m["decision_errors"] == 0.0
    assert m["delay_sign_skew_s"] < 2e-3 * tau  # the latch is symmetric


# ── DACs ──

def _rc_dac(bits, tau=None, skew=None):
    """Ideal binary DAC, 1 V full scale, through R into 1 pF (time constant tau;
    None drives vout directly). ``skew`` delays the MSB control through a
    matched, terminated line."""
    nodes = [f"d{i}" for i in range(bits)]
    dut = ps.Subcircuit("rcdac", nodes + ["vout"])
    msb = nodes[-1]
    if skew is not None:
        dut.T(name="skew", input_positive=msb, input_negative="0", output_positive="msbd",
              output_negative="0", Z0=50.0, TD=skew)
        dut.R(name="term", positive="msbd", negative="0", value=50.0)
        msb = "msbd"
    terms = "+".join(f"{2 ** i}*V({n})" for i, n in enumerate(nodes[:-1] + [msb]))
    dut.BV(name="dac", positive="src" if tau else "vout", negative="0", expression=f"({terms})/{2 ** bits - 1}")
    if tau:
        dut.R(name="out", positive="src", negative="vout", value=tau / 1e-12)
        dut.C(name="out", positive="vout", negative="0", value=1e-12)
    return dut, nodes


def _ramp_rc(tau, te):
    """RC response to a unit linear edge of duration te: 1 - K e^(-s/tau) once s > te."""
    k = tau / te * math.expm1(te / tau)
    def resp(s):
        s = np.asarray(s, float)
        during = (s - tau * -np.expm1(-np.clip(s, 0, None) / tau)) / te
        return np.where(s <= 0, 0.0, np.where(s <= te, during, 1 - k * np.exp(-s / tau)))
    return resp, k


def test_dac_transition_rc_settling_delay_slew_and_carry_glitch():
    bits, tau, hold = 6, 10e-9, 1e-6
    te, top, lsb = hold / 1000, 2 ** bits - 1, 1 / (2 ** bits - 1)
    dut, nodes = _rc_dac(bits, tau)
    m = dac_transition(ps, dut, code_nodes=nodes, hold_time=hold, max_step=tau / 100).measure()
    resp, k = _ramp_rc(tau, te)
    # After a linear edge of length te the RC error is K e^(-t/tau) of the step.
    td = tau * math.log(2 * k)
    assert m["settling_time_s"] == pytest.approx(tau * math.log(2 * k * top), rel=TOL_T, abs=0)
    assert m["settling_time_carry_s"] == pytest.approx(td, rel=TOL_T, abs=0)  # 1 LSB step, 0.5 LSB band
    assert m["delay_s"] == pytest.approx(td, rel=TOL_T, abs=0)
    assert m["slew_rate_v_per_s"] == pytest.approx(0.8 / (tau * math.log(9)), rel=TOL_T, abs=0)
    # Area identity: net = LSB (t_d - te/2 - tau) for a unity-DC-gain RC.
    assert m["glitch_impulse_vs"] == pytest.approx(lsb * (td - te / 2 - tau), rel=TOL_T, abs=0)
    s = np.linspace(0, hold / 2, 2_000_001)
    err = np.where(s < td, resp(s), resp(s) - 1)
    abs_area = np.sum(np.abs(err[1:] + err[:-1]) / 2 * np.diff(s)) * lsb
    assert m["glitch_impulse_abs_vs"] == pytest.approx(abs_area, rel=TOL_T, abs=0)
    assert m["glitch_peak_v"] == pytest.approx(lsb / 2, rel=TOL_T, abs=0)  # 0.5 LSB either side of t_d


def test_dac_transition_skewed_msb_glitch_area():
    bits, tau, hold, skew = 6, 10e-9, 1e-6, 5e-9
    te, c, lsb = hold / 1000, 2 ** (bits - 1), 1 / (2 ** bits - 1)
    dut, nodes = _rc_dac(bits, tau, skew=skew)
    m = dac_transition(ps, dut, code_nodes=nodes, hold_time=hold, max_step=tau / 100).measure()
    resp, k = _ramp_rc(tau, te)
    # Full-scale rise: c-1 LSB at t_e, c LSB at t_e + skew; 50 % crossing after both edges.
    td = tau * math.log(2 * k * ((c - 1) + c * math.exp(skew / tau)) / (2 * c - 1))
    assert td > skew + te
    assert m["delay_s"] == pytest.approx(td, rel=TOL_T, abs=0)
    # Area identity with the MSB late by skew: net = LSB (t_d - c skew - te/2 - tau).
    assert m["glitch_impulse_vs"] == pytest.approx(lsb * (td - c * skew - te / 2 - tau), rel=TOL_T, abs=0)
    s = np.linspace(0, hold / 2, 2_000_001)
    y = -(c - 1) * resp(s) + c * resp(s - skew)  # LSB, relative to the old code
    peak = np.abs(np.where(s < td, y, y - 1)).max() * lsb
    assert m["glitch_peak_v"] == pytest.approx(peak, rel=TOL_PEAK, abs=0)


def test_dac_dynamic_settled_samples_equal_the_code_spectrum():
    bits, record = 6, 256
    # No RC: a 1 ps RC under 1 ns steps rings on the trapezoidal rule (4e-3 LSB).
    dut, nodes = _rc_dac(bits)
    m = dac_dynamic(ps, dut, code_nodes=nodes, record=record, samples_per_update=1).measure()
    j = coherent_bin(record, 0.1)
    # The bench plays n = 0.. and reads from n = 8 (settle_updates); index the
    # same n, since round() at the x.5 zero crossings tips on the last bit of sin.
    n = 8 + np.arange(record)
    codes = np.round(31.5 * (1 + np.sin(2 * np.pi * j * n / record)))
    sinad, a_fit = _sinad_by_fit(codes, j)
    # K = 1 reads flat held values: the record is the code sequence, to float rounding.
    assert m["sndr_db"] == pytest.approx(sinad, abs=1e-6)
    assert m["fundamental_dbfs"] == pytest.approx(20 * math.log10(a_fit / 31.5), abs=1e-6)


def test_dac_dynamic_oversampled_reads_see_the_hold_droop():
    bits, record, kk = 6, 256, 8
    dut, nodes = _rc_dac(bits)
    m = dac_dynamic(ps, dut, code_nodes=nodes, record=record, samples_per_update=kk).measure()
    j = coherent_bin(record, 0.1)
    codes = np.round(31.5 * (1 + np.sin(2 * np.pi * j * (8 + np.arange(record)) / record)))
    # K equal reads per update: bin b of the code DFT scales by the discrete
    # hold kernel |sin(pi b/M) / (K sin(pi b/(K M)))|. Exact for flat reads.
    c = np.abs(np.fft.rfft(codes))[1:]
    b = np.arange(1, record // 2 + 1)
    kernel = np.abs(np.sin(np.pi * b / record) / (kk * np.sin(np.pi * b / (kk * record))))
    p = (c * kernel) ** 2
    p[-1] /= 4  # code-DFT Nyquist bin is unmirrored; in the K*M record bin M/2 is an ordinary bin
    sndr = 10 * np.log10(p[j - 1] / (p.sum() - p[j - 1]))
    _, a_fit = _sinad_by_fit(codes, j)
    assert m["fundamental_dbfs"] == pytest.approx(20 * math.log10(a_fit * kernel[j - 1] / 31.5), abs=1e-6)
    assert m["sndr_db"] == pytest.approx(sndr, abs=1e-6)


# ── sample and hold ──

def _switch_sampler(ron, roff=1e12, coff=None):
    dut = ps.Subcircuit("sh", ["vin", "vhold", "phi"])
    dut.raw_spice(f".model swh SW(ron={ron} roff={roff} vt=0.5 vh=0)")
    dut.S(name="s", positive="vin", negative="vhold", control_positive="phi", control_negative="0", model="swh")
    if coff is not None:
        dut.C(name="off", positive="vin", negative="vhold", value=coff)
    return dut


def test_sample_hold_dynamic_rc_sampled_gain():
    ron, ch, fs, record = 2e3, 1e-12, 10e6, 256
    m = sample_hold_dynamic(ps, _switch_sampler(ron), hold_capacitance=ch, sample_frequency=fs,
                            record=record).measure()
    fin = coherent_bin(record, 0.45) * fs / record
    gain = -10 * math.log10(1 + (2 * math.pi * fin * ron * ch) ** 2)
    assert gain < -0.01, "the RC must attenuate measurably"
    # Track lasts 25 RC (residue e^-25 = 1e-11); the rest is solver error.
    assert m["sampled_gain_db"] == pytest.approx(gain, abs=TOL_SH_DB)
    assert m["sndr_db"] > SH_FLOOR_DB  # a linear sampler: only the bench's numerics remain


def _conductance_sampler(ron, roff, coff=None):
    """The switch as a B-source conductance: ngspice-45's SW element is on in
    .op yet stamps roff in .ac, so an S-switch DUT reads its off state there."""
    dut = ps.Subcircuit("shg", ["vin", "vhold", "phi"])
    dut.BI(name="s", positive="vin", negative="vhold",
           expression=f"(V(vin)-V(vhold))*({1 / ron}*u(V(phi)-0.5)+{1 / roff})")
    if coff is not None:
        dut.C(name="off", positive="vin", negative="vhold", value=coff)
    return dut


def test_sample_hold_track_bandwidth_is_the_rc_corner():
    ron, ch = 2e3, 1e-12
    m = sample_hold_track_bandwidth(ps, _conductance_sampler(ron, 1e12), hold_capacitance=ch).measure()
    # Log interpolation at 50 points/decade: 1e-4 at 100/decade (test_bench_metrics),
    # error ~ 1/ppd^2, so 4e-4; 1e-3.
    assert m["track_bandwidth_hz"] == pytest.approx(1 / (2 * math.pi * ron * ch), rel=1e-3, abs=0)
    assert m["track_gain_db"] == pytest.approx(0.0, abs=1e-6)  # 1 kHz is 1e-10 dB under DC


def test_sample_hold_feedthrough_is_the_capacitive_divider():
    coff, ch = 1e-15, 1e-12
    dut = _conductance_sampler(1.0, 1e15, coff=coff)
    m = sample_hold_feedthrough(ps, dut, hold_capacitance=ch).measure()
    # C_off into C_H, flat while 1/(w C_off) << R_off (1.6e11 << 1e15 ohm at 1 kHz).
    expected = 20 * math.log10(coff / (coff + ch))
    assert m["feedthrough_db"] == pytest.approx(expected, abs=0.01)
    assert m["feedthrough_max_db"] == pytest.approx(expected, abs=0.01)


# ── architecture-specific ──

def _kick_dut(cgs, dvq):
    """Each input sees C_gs to a node driven -dvq while clk is high: Q = C_gs dvq per edge."""
    dut = ps.Subcircuit("kick", ["inp", "inn", "clk"])
    for side in ("p", "n"):
        dut.E(name=f"q{side}", positive=f"q{side}", negative="0", control_positive="clk",
              control_negative="0", voltage_gain=-dvq)
        dut.C(name=f"gs{side}", positive=f"in{side}", negative=f"q{side}", value=cgs)
    return dut


def test_comparator_kickback_charge_and_peak():
    cgs, dvq, cs, rp, rn, period = 50e-15, 0.5, 200e-15, 10e3, 1e3, 100e-9
    te, ct = period / 1000, cs + cgs
    m = comparator_kickback(ps, _kick_dut(cgs, dvq), overdrive=0.0, source_resistance_p=rp,
                            source_resistance_n=rn, source_capacitance=cs, clock_period=period,
                            cycles=2, max_step=rn * ct / 50).measure()
    dv = cgs * dvq / ct  # charge sharing, an instant edge
    # A kick spread over the clock edge te decays during it: e(s) = dv (tau/te)(1 - e^(-s/tau)) up to te.
    ramp = lambda tau, s: dv * tau / te * -np.expm1(-np.minimum(s, te) / tau) * np.exp(-np.clip(s - te, 0, None) / tau)
    tau_p, tau_n = rp * ct, rn * ct
    # Half period = 20 tau_p: the charge returns to within e^-20 of C_gs dvq.
    assert m["kick_charge_c"] == pytest.approx(cgs * dvq, rel=1e-4, abs=0)
    assert m["kick_peak_v"] == pytest.approx(ramp(tau_p, te), rel=1e-4, abs=0)
    s = np.linspace(0, 10 * tau_p, 1_000_001)
    assert m["kick_diff_peak_v"] == pytest.approx(np.abs(ramp(tau_p, s) - ramp(tau_n, s)).max(), rel=1e-4, abs=0)


def test_comparator_kickback_residual_at_the_sampling_edge():
    cgs, dvq, cs, rp, period = 50e-15, 0.5, 200e-15, 10e3, 10e-9
    m = comparator_kickback(ps, _kick_dut(cgs, dvq), overdrive=0.0, source_resistance_p=rp,
                            source_capacitance=cs, clock_period=period, cycles=8).measure()
    te, ct = period / 1000, cs + cgs
    tau = rp * ct
    k = tau / te * math.expm1(te / tau)  # after a ramp kick of te: dv K e^(-s/tau)
    dv = cgs * dvq / ct
    # Steady state just before the rising edge at (k+1/2)T: +kick from the
    # falling edge at kT (age T/2), -kick from the last rising edge (age T - te,
    # its PWL edge starts te late), and every earlier cycle times e^(-T/tau).
    residual = dv * k * (math.exp(-period / 2 / tau) - math.exp(-(period - te) / tau)) / -math.expm1(-period / tau)
    assert residual > 1e-3, "the residual must be a non-degenerate kick tail"
    # The last of eight cycles: the series is complete to e^(-28); the n side
    # (100 ohm, 25 ps) has decayed. The first edge, with no history, reads 0.0136.
    assert m["kick_residual_v"] == pytest.approx(residual, rel=1e-3, abs=0)


def test_pipeline_stage_residue_recovers_gain_and_threshold_offset():
    g, vos, vref, levels = 1.98, 10e-3, 0.5, 257
    dut = ps.Subcircuit("mdac", ["vin", "vres", "phi1", "phi2"])
    hi, lo = vref / 4 + vos, -vref / 4 + vos
    dut.BV(name="r", positive="vres", negative="0",
           expression=f"{g}*V(vin)-{vref}*(u(V(vin)-{hi})-u({lo}-V(vin)))")
    m = pipeline_stage_residue(ps, dut, vref=vref, levels=levels).measure()
    v = np.linspace(-vref, vref, levels)
    d = (v > hi).astype(float) - (v < lo)
    step = 2 * vref / (levels - 1)
    assert m["interstage_gain"] == pytest.approx(g, rel=1e-9, abs=0)  # piecewise linear: exact
    assert m["gain_error"] == pytest.approx(g / 2 - 1, abs=1e-9)
    assert m["residue_max_v"] == pytest.approx(np.abs(g * v - d * vref).max(), abs=1e-9)
    assert m["overrange_margin_v"] == pytest.approx(vref - np.abs(g * v - d * vref).max(), abs=1e-9)
    # Both thresholds move by vos; each is reported as the midpoint of the two
    # levels around it, which pins the error exactly.
    found = [v[np.searchsorted(v, x) - 1] + step / 2 for x in (lo, hi)]
    assert m["threshold_error_max_v"] == pytest.approx(max(abs(found[0] + vref / 4), abs(found[1] - vref / 4)), abs=1e-12)
    assert m["threshold_error_max_v"] == pytest.approx(vos, abs=step / 2)
    assert m["residue_jump_v"] == pytest.approx(vref, abs=1e-9)


def test_dac_output_impedance_of_switched_unit_conductances():
    bits, code, ru, rl, cout = 4, 5, 10e3, 25.0, 1e-12
    nodes = [f"d{i}" for i in range(bits)]
    dut = ps.Subcircuit("idac", nodes + ["vout"])
    # Bit i switches 2^i units, each 1 mA in parallel with R_u, onto the output.
    terms = "+".join(f"V({n})*{2 ** i}*(1e-3+V(vout)/{ru})" for i, n in enumerate(nodes))
    dut.BI(name="cells", positive="vout", negative="0", expression=terms)
    dut.C(name="out", positive="vout", negative="0", value=cout)
    m = dac_output_impedance(ps, dut, code_nodes=nodes, code=code, load_resistance=rl).measure()
    r = 1 / (code / ru + 1 / rl)  # (R_u / code) || R_L
    fc = 1 / (2 * math.pi * r * cout)
    assert m["zout_dc_ohm"] == pytest.approx(r, rel=1e-6, abs=0)  # 1 kHz is (1e3/fc)^2 = 2e-14 below DC
    assert m["zout_at_f_ohm"] == pytest.approx(r / math.hypot(1, 1e8 / fc), rel=1e-4, abs=0)  # log interp at 50/decade
    assert m["zout_corner_hz"] == pytest.approx(fc, rel=1e-3, abs=0)  # as track_bandwidth: 50 points/decade
