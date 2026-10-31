"""Amplifier benches on ngspice against behavioural DUTs with closed-form answers.

Every DUT is E, G, C (and B where a nonlinearity is the point), with resistors
written as G elements that sense their own terminals: no noise unless a test
wants it. Each expected value is a closed form evaluated here with numpy,
never the extractor's own method.
"""
import math

import numpy as np
import pytest

import spicerack as ps
from spicerack.testbenches.amplifiers import (
    PP_PER_RMS, loop_gain, noise_spectrum, opamp_open_loop, rf_compression_ip3, step_response,
)
from spicerack.testbenches.amplifiers_2 import (
    cmrr, harmonic_distortion, offset_icmr, output_swing, port_impedance, rf_two_port,
)

K_B, T_SIM = 1.380649e-23, 300.15  # ngspice default 27 C


def _rel(x, rel):
    """pytest.approx without its default abs=1e-12, which would swallow a pF or a nV/rtHz."""
    return pytest.approx(x, rel=rel, abs=0.0)


def _gres(d, name, a, b, r):
    """Noiseless resistor: a VCCS across its own terminals."""
    d.G(name=f"r{name}", positive=a, negative=b, control_positive=a, control_negative=b, transconductance=1.0 / r)


def _pole(d, name, ctrl_p, ctrl_n, node, gain, f_pole, gm=1e-3):
    """gm*v(ctrl) into `node`, loaded by a noiseless R || C: gain/(1 + s/(2 pi f_pole))."""
    d.G(name=name, positive=d.gnd, negative=node, control_positive=ctrl_p, control_negative=ctrl_n,
        transconductance=gm)
    _gres(d, name, node, d.gnd, gain / gm)
    d.C(name=name, positive=node, negative=d.gnd, value=gm / (gain * 2 * math.pi * f_pole))


def _opamp(name, a0, poles, ro=None, d=None, inp="inp", inn="inn", out="out"):
    """Op-amp A(s) = a0 / prod(1 + s/w_k), out = E(last stage) behind an optional noiseless Ro.

    Built into ``d`` when given (a closed-loop DUT), else into a new subcircuit."""
    d = d or ps.Subcircuit(name, [inp, inn, out])
    _pole(d, "1", inp, inn, "n1", a0, poles[0])
    for k, fp in enumerate(poles[1:], start=2):
        _pole(d, str(k), f"n{k - 1}", d.gnd, f"n{k}", 1.0, fp)
    last = f"n{len(poles)}"
    if ro is None:
        d.E(name="o", positive=out, negative=d.gnd, control_positive=last, control_negative=d.gnd, voltage_gain=1.0)
    else:
        d.E(name="o", positive="no", negative=d.gnd, control_positive=last, control_negative=d.gnd, voltage_gain=1.0)
        _gres(d, "o", "no", out, ro)
    return d


def _a(f, a0, poles):
    s = 1j * np.asarray(f, float)
    return a0 / np.prod([1 + s / fp for fp in poles], axis=0)


def _root(fn, lo, hi):
    """x in [lo, hi] (log-bisected) where fn changes sign."""
    flo = fn(lo)
    for _ in range(200):
        mid = math.sqrt(lo * hi)
        if (fn(mid) > 0) == (flo > 0):
            lo, flo = mid, fn(mid)
        else:
            hi = mid
    return math.sqrt(lo * hi)


def _ugf_pm(dc, poles):
    """UGF and PM of dc / prod(1 + s/p): real poles, so the phase is -sum(atan(f/p))."""
    ugf = _root(lambda x: abs(complex(_a(x, dc, poles))) - 1.0, 1e-3, 1e12)
    return ugf, 180.0 - sum(math.degrees(math.atan(ugf / p)) for p in poles)


# ── opamp_open_loop ──

@pytest.mark.parametrize("poles", [(100.0, 50e6), (100.0, 150e3, 500e3)])  # PM +76 deg, -10 deg
def test_open_loop_matches_the_pole_product(poles):
    a0 = 1e4
    bench = opamp_open_loop(ps, _opamp("ol", a0, poles), load_capacitance=None)
    got = bench.measure()
    t = lambda f: complex(_a(f, a0, poles))
    f_p1 = _root(lambda x: abs(t(x)) - a0 / math.sqrt(2), 1.0, 1e9)
    ugf, pm = _ugf_pm(a0, poles)
    # Servo leak at 0.1 Hz: |loop gain| a0/(w^2 LC) = 2.5e-5 moves A0 by 2.2e-4 dB.
    assert got["a0_db"] == pytest.approx(80.0, abs=1e-3)
    # f_p1 is referenced to A(f_min), so the 2.2e-4 dB servo error shifts it 5e-5 at any density.
    assert got["f_p1_hz"] == _rel(f_p1, 2e-4)
    assert got["gbw_hz"] == _rel(a0 * f_p1, 2e-4)
    # ln|A| linear in ln f between points 1/50 decade apart: measured 8e-5 (5e-6 at 200/dec).
    assert got["ugf_hz"] == _rel(ugf, 2e-4)
    # Same chord on the phase: measured 5e-3 deg at 50/dec (3e-4 at 200/dec).
    assert got["pm_unity_deg"] == pytest.approx(pm, abs=0.02)
    if len(poles) == 2:
        assert math.isnan(got["gm_unity_db"])  # two poles never reach -180 deg
    else:
        assert pm < 0  # unstable at unity feedback: the wrap defect would read 360 + pm
        pcf = _root(lambda x: sum(math.atan(x / p) for p in poles) - math.pi, 1.0, 1e9)
        assert got["gm_unity_db"] == pytest.approx(-20 * math.log10(abs(t(pcf))), abs=0.005)  # chord: measured 6.5e-4 dB


# ── loop_gain ──

def test_loop_gain_through_a_loaded_break_matches_the_return_ratio():
    """Non-inverting gain of 2 broken at inn: e carries Cin, f carries Rg, the amp drives through Ro.

    T = A(s) * Zp / (Ro + Rf + Zp) with Zp = Rg || 1/(s Cin): one more real pole."""
    a0, poles, ro, rf, rg, cin = 1e4, (100.0, 5e6), 1e3, 10e3, 10e3, 10e-12
    d = ps.Subcircuit("lg", ["e", "f"])
    _opamp("", a0, poles, ro=ro, d=d, inp=d.gnd, inn="e", out="out")
    _gres(d, "f", "out", "f", rf)
    _gres(d, "g", "f", d.gnd, rg)
    d.C(name="in", positive="e", negative=d.gnd, value=cin)
    got = loop_gain(ps, d, e="e", f="f").measure()
    dc = a0 * rg / (ro + rf + rg)
    all_poles = (*poles, (ro + rf + rg) / (2 * math.pi * cin * rg * (ro + rf)))
    ugf, pm = _ugf_pm(dc, all_poles)
    pcf = _root(lambda x: sum(math.atan(x / p) for p in all_poles) - math.pi, 1.0, 1e10)
    # |T(f_min)| exactly; Tian's algebra is a linear solve, so only solver precision (1e-9 rel).
    assert got["t0_db"] == pytest.approx(20 * math.log10(abs(complex(_a(1.0, dc, all_poles)))), abs=1e-6)
    # Chord interpolation at 50/dec, as in the open-loop test: measured ugf 1.6e-5, pm 3.3e-3 deg,
    # crossover 2.8e-7, gm 2.1e-3 dB; each shrinks 15-30x at 200/dec.
    assert got["ugf_hz"] == _rel(ugf, 1e-4)
    assert got["pm_deg"] == pytest.approx(pm, abs=0.02)
    assert got["phase_crossover_hz"] == _rel(pcf, 1e-5)
    assert got["gm_db"] == pytest.approx(-20 * math.log10(abs(complex(_a(pcf, dc, all_poles)))), abs=0.005)


# ── step_response ──

def _follower(name, a0, poles):
    d = ps.Subcircuit(name, ["vin", "vout"])
    return _opamp("", a0, poles, d=d, inp="vin", inn="vout", out="vout")


def test_step_settling_of_a_one_pole_follower():
    """Closed-loop pole wc = 2 pi f1 (1 + a0). For a ramp input of length e the error after
    the ramp is step * (tau/e)(exp(e/tau) - 1) exp(-t/tau), t from the ramp start."""
    a0, f1, period = 1e4, 1e3, 2e-6
    tau = 1 / (2 * math.pi * f1 * (1 + a0))
    max_step = tau / 100
    bench = step_response(ps, _follower("f1", a0, (f1,)), period=period, max_step=max_step)
    got = bench.measure()
    e = period / 1000
    for eps, name in ((1e-3, "0p1pct"), (1e-4, "0p01pct")):
        want = tau * math.log(tau / e * math.expm1(e / tau) / eps) - e / 2
        for edge in ("rise", "fall"):
            # Trapezoidal steps of tau/100: measured 5e-5 tau, falling 16x at tau/400 (O(h^2)).
            assert got[f"settle_{edge}_{name}_s"] == pytest.approx(want, abs=1e-3 * tau), (edge, name)
    gain = a0 / (1 + a0)
    # A linear DC solve: measured 5e-14 V.
    assert got["v_final"] == pytest.approx(gain, abs=1e-9)
    assert got["static_error_pct"] == pytest.approx((gain - 1) * 100, abs=1e-7)
    # 20 % and 80 % fall after the ramp ends, on a pure exponential: dt = tau ln 4 (measured 1e-5).
    for edge in ("rise", "fall"):
        assert got[f"slew_{edge}_v_per_s"] == _rel(0.6 * gain / (tau * math.log(4)), 1e-4)
    assert got["overshoot_rise_pct"] == pytest.approx(0.0, abs=1e-9)


def test_step_overshoot_of_a_two_pole_follower():
    """wn^2 = w1 w2 (1 + a0), 2 zeta wn = w1 + w2, overshoot = exp(-pi zeta / sqrt(1 - zeta^2))."""
    a0, f1, f2 = 1e4, 100.0, 640e3
    zeta = (f1 + f2) / (2 * math.sqrt(f1 * f2 * (1 + a0)))
    wn = 2 * math.pi * math.sqrt(f1 * f2 * (1 + a0))
    got = step_response(ps, _follower("f2", a0, (f1, f2)), period=10e-6, max_step=1 / (200 * wn)).measure()
    want = 100 * math.exp(-math.pi * zeta / math.sqrt(1 - zeta**2))
    # The 10 ns input ramp lowers the peak by (wd e)^2/24 = 9e-5 of 25 % = 2.2e-3 pp (measured
    # 2.6e-3); the final value sits 19 envelope constants in, so its residual is 6e-9.
    for edge in ("rise", "fall"):
        assert got[f"overshoot_{edge}_pct"] == pytest.approx(want, abs=0.005)


def test_step_slew_of_an_asymmetric_tanh_stage():
    """I = Imax * tanh(gm vd / Imax) into C, Imax = 10 uA rising and 5 uA falling: SR = Imax / C."""
    ip, i_n, gm, c = 10e-6, 5e-6, 1e-3, 1e-12
    d = ps.Subcircuit("sl", ["vin", "vout"])
    vd = "(v(vin)-v(vout))"
    d.BI(name="1", positive=d.gnd, negative="n1",
         expression=f"{vd} > 0 ? {ip}*tanh({gm}*{vd}/{ip}) : {i_n}*tanh({gm}*{vd}/{i_n})")
    _gres(d, "1", "n1", d.gnd, 1e12)
    d.C(name="1", positive="n1", negative=d.gnd, value=c)
    d.E(name="o", positive="vout", negative=d.gnd, control_positive="n1", control_negative=d.gnd, voltage_gain=1.0)
    got = step_response(ps, d, period=1e-6, max_step=1e-10).measure()
    # In the 20-80 % window gm vd / Imax >= 20, so tanh = 1 - 1e-17; the 1 TOhm leak
    # takes 0.5e-12 A of 5 uA (1e-7, measured). The ramp is linear, so its crossings are exact.
    assert got["slew_rise_v_per_s"] == _rel(ip / c, 1e-6)
    assert got["slew_fall_v_per_s"] == _rel(i_n / c, 1e-6)


# ── noise ──

def test_noise_of_a_gain_stage_into_an_rc():
    """EMF -> Rs (noisy) -> E gain G (noiseless) -> Ro (noisy) -> out, C to ground.
    S_out = 4kT (G^2 Rs + Ro) / (1 + (f/f0)^2), H = G / (1 + j f/f0), f0 = 1/(2 pi Ro C)."""
    g, rs, ro, c = 3.0, 1e3, 1e3, 1e-12
    d = ps.Subcircuit("nz", ["vin", "vout"])
    d.E(name="a", positive="a", negative=d.gnd, control_positive="vin", control_negative=d.gnd, voltage_gain=g)
    d.R(name="o", positive="a", negative="vout", value=ro)
    d.C(name="o", positive="vout", negative=d.gnd, value=c)
    band, fmax = (1e5, 1e10), 1e11  # straddles f0 = 159 MHz, so |H| at the band centre matters
    got = noise_spectrum(ps, d, band=band, source_resistance=rs, spots=(1e3, 1e8), stop_frequency=fmax).measure()
    kt = K_B * T_SIM
    s0, f0 = 4 * kt * (g * g * rs + ro), 1 / (2 * math.pi * ro * c)
    power = lambda lo, hi: s0 * f0 * (math.atan(hi / f0) - math.atan(lo / f0))
    h = lambda f: g / abs(1 + 1j * f / f0)
    out_rms = math.sqrt(power(*band))
    # Power-law segments on a Lorentzian at 50 points/decade: measured 4.4e-5 (3e-6 at 200/dec);
    # the squared total in noise_equivalent_c doubles it.
    rel = 1e-4
    assert got["out_rms_band_v"] == _rel(out_rms, rel)
    assert got["in_rms_band"] == _rel(out_rms / h(math.sqrt(band[0] * band[1])), rel)
    assert got["out_vpp_0p1_10hz"] == _rel(PP_PER_RMS * math.sqrt(power(0.1, 10.0)), rel)
    assert got["out_rms_total_v"] == _rel(math.sqrt(power(0.1, fmax)), rel)
    assert got["noise_equivalent_c_f"] == _rel(kt / power(0.1, fmax), 2 * rel)
    for f, key in ((1e3, "1khz"), (1e8, "100mhz")):
        # Spots land on sweep points; ngspice's k is 3.5e-7 off CODATA (measured 1.7e-7 on a density).
        assert got[f"out_density_at_{key}"] == _rel(math.sqrt(s0) / abs(1 + 1j * f / f0), 1e-6)
        assert got[f"in_density_at_{key}"] == _rel(math.sqrt(4 * kt * (rs + ro / g**2)), 1e-6)


# ── cmrr ──

def test_cmrr_of_a_macromodel_with_a_cm_leak_and_cm_capacitance():
    """Stage 1 takes gm vd + (gcm + s Ccm) vcm into one node, so CMRR = gm / |gcm + j w Ccm|.
    vcm comes from two stacked E halves; s Ccm vcm is the current of a 0 V source behind Ccm,
    mirrored into the node by an F."""
    gm, gcm, ccm, a0, poles = 1e-3, 1e-7, 1e-7 / (2 * math.pi * 1e4), 1e4, (100.0, 5e6)
    d = _opamp("cm", a0, poles)
    d.E(name="c1", positive="x", negative=d.gnd, control_positive="inp", control_negative=d.gnd, voltage_gain=0.5)
    d.E(name="c2", positive="cmn", negative="x", control_positive="inn", control_negative=d.gnd, voltage_gain=0.5)
    d.G(name="cm", positive=d.gnd, negative="n1", control_positive="cmn", control_negative=d.gnd, transconductance=gcm)
    d.C(name="cm", positive="cmn", negative="dcm", value=ccm)
    d.V(name="s", positive="dcm", negative=d.gnd, value=0.0)
    d.F(name="cm", positive=d.gnd, negative="n1", vsense="Vs", current_gain=1.0)
    got = cmrr(ps, d, spots=(1e3, 1e5)).measure()
    want = lambda f: 20 * math.log10(gm / abs(gcm + 2j * math.pi * f * ccm))
    # Both runs share stage 1's load and every later pole, so the ratio is exact to solver
    # precision (measured 1e-11 dB); spots land on sweep points.
    assert got["cmrr_dc_db"] == pytest.approx(want(1.0), abs=1e-6)
    assert got["cmrr_db_at_1khz"] == pytest.approx(want(1e3), abs=1e-6)
    assert got["cmrr_db_at_100khz"] == pytest.approx(want(1e5), abs=1e-6)
    a_cm = abs((gcm + 2j * math.pi * ccm) / gm * complex(_a(1.0, a0, poles)))
    # The servo leak at 1 Hz, |A| / (w^2 LC) = 2.5e-7, moves the raw gain 2.2e-6 dB.
    assert got["a_cm_db"] == pytest.approx(20 * math.log10(a_cm), abs=1e-5)
    # -3.0103 dB from the 1 Hz value (4e-8 dB below DC), log-interpolated: measured 1e-8.
    assert got["cmrr_corner_hz"] == _rel(1e4, 1e-6)


def test_cmrr_of_a_difference_amp_without_servo():
    """Op-amp gain 1e6, R1 = R3 = 10k, R2 = 100k, R4 = 100k (1 + 1e-3): out = a v(inp) - b v(inn),
    so A_dm = (a + b)/2 and A_cm = a - b."""
    r1, r2, r3, r4, gain = 10e3, 100e3, 10e3, 100e3 * (1 + 1e-3), 1e6
    d = ps.Subcircuit("da", ["inp", "inn", "out"])
    _gres(d, "1", "inn", "m", r1)
    _gres(d, "2", "m", "out", r2)
    _gres(d, "3", "inp", "p", r3)
    _gres(d, "4", "p", d.gnd, r4)
    d.E(name="a", positive="out", negative=d.gnd, control_positive="p", control_negative="m", voltage_gain=gain)
    got = cmrr(ps, d, servo=False, stop_frequency=1e6).measure()
    # Exact for op-amp gain A: out = A (v(inp) R4/(R3+R4) - v(inn) R2/(R1+R2)) / (1 + A R1/(R1+R2)).
    loop = 1 + gain * r1 / (r1 + r2)
    a, b = gain * r4 / (r3 + r4) / loop, gain * r2 / (r1 + r2) / loop
    # A linear solve, but A_cm cancels two terms of 10 down to 9e-4: measured 5e-8 dB.
    assert got["cmrr_dc_db"] == pytest.approx(20 * math.log10(abs(a + b) / (2 * abs(a - b))), abs=1e-6)
    assert got["a_cm_db"] == pytest.approx(20 * math.log10(abs(a - b)), abs=1e-6)
    assert math.isnan(got["cmrr_corner_hz"])  # resistive: flat to the sweep end


# ── offset_icmr ──

C_MID, W_IN, VOS, A_FOLLOW = 0.9, 0.3, 1e-3, 1e4


def _icmr_amp():
    """out = A (g(v(inp) + Vos) - v(inn)), g(x) = c + w tanh((x - c)/w): a follower's gain is
    sech^2(.) A/(1+A). Ib = 100 nA into inp, 80 nA into inn."""
    d = ps.Subcircuit("icmr", ["inp", "inn", "out"])
    d.V(name="os", positive="inpi", negative="inp", value=VOS)
    d.I(name="bp", positive="inp", negative=d.gnd, value=100e-9)
    d.I(name="bn", positive="inn", negative=d.gnd, value=80e-9)
    d.BV(name="g", positive="n2", negative=d.gnd, expression=f"{C_MID}+{W_IN}*tanh((v(inpi)-{C_MID})/{W_IN})")
    d.E(name="a", positive="out", negative=d.gnd, control_positive="n2", control_negative="inn",
        voltage_gain=A_FOLLOW)
    return d


def test_offset_bias_current_and_icmr_of_a_follower():
    tol, step = 0.01, 0.01
    got = offset_icmr(ps, _icmr_amp(), sweep=(0.0, 1.8), vcm=C_MID, step=step, tol_gain=tol).measure()
    k = A_FOLLOW / (1 + A_FOLLOW)
    g = lambda x: C_MID + W_IN * math.tanh((x - C_MID) / W_IN)
    # A DC solve at vntol 1 nV: the offset is exact to ~1e-9 V.
    assert got["vos_v"] == pytest.approx(g(C_MID + VOS) * k - C_MID, abs=1e-8)
    assert got["ib_pos_a"] == _rel(100e-9, 1e-6)
    assert got["ib_neg_a"] == _rel(80e-9, 1e-6)
    assert got["ios_a"] == _rel(20e-9, 1e-5)
    # sech^2(u) k = 1 - tol at u = acosh(sqrt(k / (1 - tol))).
    u = math.acosh(math.sqrt(k / (1 - tol)))
    # Central differences on a 10 mV grid err by h^2/6 f''' = 3.7e-4 in gain, where the gain falls
    # 0.66 /V: the edge moves 5.6e-4 V (measured 6.0e-4 low, 8.0e-4 high; 4x less at 5 mV).
    assert got["icmr_low_v"] == pytest.approx(C_MID - VOS - W_IN * u, abs=1e-3)
    assert got["icmr_high_v"] == pytest.approx(C_MID - VOS + W_IN * u, abs=1e-3)


# ── port_impedance ──

@pytest.mark.parametrize("drive", ["current", "voltage"])
def test_port_impedance_of_a_parallel_rlc(drive):
    """Y = 1/R + j w C + 1/(j w L): |Z| peaks at exactly R at f0 = 1/(2 pi sqrt(LC))."""
    r, ind, c = 1e3, 10e-6, 100e-12
    d = ps.Subcircuit("rlc", ["p"])
    d.R(name="1", positive="p", negative=d.gnd, value=r)
    d.L(name="1", positive="p", negative=d.gnd, value=ind)
    d.C(name="1", positive="p", negative=d.gnd, value=c)
    spot_f = 100e6  # on the 50/dec grid
    got = port_impedance(ps, d, port="p", drive=drive, spots=(spot_f,)).measure()
    z = lambda f: 1 / (1 / r + 2j * math.pi * f * c + 1 / (2j * math.pi * f * ind))
    f0 = 1 / (2 * math.pi * math.sqrt(ind * c))
    # Spots land on sweep points and DC is f_min = 1 Hz: a linear solve, 1e-9 relative.
    assert got["z_dc_ohm"] == _rel(abs(z(1.0)), 1e-9)
    assert got["r_parallel_ohm"] == _rel(r, 1e-8)  # 1/R is 6e-8 of |Y| at 1 Hz: 1.3e-9
    assert got["c_parallel_f"] == _rel(c - 1 / ((2 * math.pi * spot_f) ** 2 * ind), 1e-9)
    assert got["z_ohm_at_100mhz"] == _rel(abs(z(spot_f)), 1e-9)
    assert got["z_phase_deg_at_100mhz"] == pytest.approx(math.degrees(np.angle(z(spot_f))), abs=1e-9)
    # A parabola in (ln f, ln|Z|) through 3 points 1/50 decade apart around a Q = 3.2 peak:
    # measured 3.9e-5 on |Z| and 1.6e-4 on f (2.4e-6 and 5.1e-6 at 200/dec).
    assert got["z_peak_ohm"] == _rel(r, 1e-4)
    assert got["z_peak_hz"] == _rel(f0, 5e-4)


# ── output_swing ──

def test_output_swing_of_a_tanh_output_stage():
    """out = Vmax tanh(A0 vd / Vmax): A_ol = A0 (1 - (out/Vmax)^2), so the gain is 10 % down at
    |out| = Vmax sqrt(0.1)."""
    a0, vmax = 1e3, 1.0
    d = ps.Subcircuit("sw", ["inp", "inn", "out"])
    d.E(name="a", positive="n2", negative=d.gnd, control_positive="inp", control_negative="inn", voltage_gain=a0)
    d.BV(name="o", positive="out", negative=d.gnd, expression=f"{vmax}*tanh(v(n2)/{vmax})")
    got = output_swing(ps, d, vref=0.0, sweep=(-2.0, 2.0)).measure()
    edge = vmax * math.sqrt(0.1)
    # Central differences of two smooth curves on a 1 mV grid plus a linear crossing:
    # measured 4.5e-7 V (2.0e-7 at 0.5 mV); the peak sample sits at out = 0, 2.9e-6 dB.
    assert got["vout_max_v"] == pytest.approx(edge, abs=2e-6)
    assert got["vout_min_v"] == pytest.approx(-edge, abs=2e-6)
    assert got["swing_pp_v"] == pytest.approx(2 * edge, abs=4e-6)
    assert got["a_ol_peak_db"] == pytest.approx(60.0, abs=1e-5)


# ── harmonic_distortion ──

def test_harmonics_of_a_memoryless_cubic():
    """y = a1 x + a2 x^2 + a3 x^3, x = A sin: |H1| = a1 A + 3/4 a3 A^3, |H2| = a2 A^2 / 2,
    |H3| = |a3| A^3 / 4, nothing above the third."""
    a1, a2, a3, amp = 1.0, 0.05, -0.02, 0.5
    d = ps.Subcircuit("hd", ["vin", "vout"])
    d.BV(name="o", positive="vout", negative=d.gnd, expression=f"{a1}*v(vin)+{a2}*v(vin)*v(vin)+{a3}*v(vin)*v(vin)*v(vin)")
    got = harmonic_distortion(ps, d, amplitude=amp).measure()
    h1, h2, h3 = a1 * amp + 0.75 * a3 * amp**3, a2 * amp**2 / 2, abs(a3) * amp**3 / 4
    # Samples interpolated between points 1/(16 fs) apart: measured 6.5e-5 dB on HD3
    # (4.1e-6 at 64 steps per sample, O(h^2)).
    assert got["hd2_dbc"] == pytest.approx(20 * math.log10(h2 / h1), abs=2e-4)
    assert got["hd3_dbc"] == pytest.approx(20 * math.log10(h3 / h1), abs=2e-4)
    thd = 10 * math.log10((h2**2 + h3**2) / h1**2)
    assert got["thd_db"] == pytest.approx(thd, abs=2e-4)
    assert got["thd_pct"] == _rel(100 * 10 ** (thd / 20), 1e-4)
    assert got["sfdr_dbc"] == pytest.approx(-20 * math.log10(h2 / h1), abs=2e-4)
    assert got["sndr_db"] == pytest.approx(-thd, abs=2e-4)  # no noise: SINAD is 1/THD


# ── rf_two_port ──

def test_s_parameters_and_noise_figure_of_a_resistive_l_section():
    """Series R1 then shunt R2 at port 2. S from the ABCD matrix [[1 + R1/R2, R1], [1/R2, 1]];
    with a noiseless load F = 1 + R1/Z0 + (Z0 + R1)^2 / (R2 Z0) at the simulation temperature."""
    r1, r2, z0, t_sim = 30.0, 200.0, 50.0, T_SIM
    d = ps.Subcircuit("lsec", ["p1", "p2"])
    d.R(name="1", positive="p1", negative="p2", value=r1)
    d.R(name="2", positive="p2", negative=d.gnd, value=r2)
    got = rf_two_port(ps, d, frequency=1e8).measure()
    a, b, c, dd = 1 + r1 / r2, r1, 1 / r2, 1.0
    den = a + b / z0 + c * z0 + dd
    s = {"s11": (a + b / z0 - c * z0 - dd) / den, "s12": 2 * (a * dd - b * c) / den,
         "s21": 2 / den, "s22": (-a + b / z0 - c * z0 + dd) / den}
    for k, v in s.items():
        # A linear solve at a sweep point: measured 2e-14 dB.
        assert got[f"{k}_db"] == pytest.approx(20 * math.log10(abs(v)), abs=1e-9), k
    f_sim = 1 + r1 / z0 + (z0 + r1) ** 2 / (r2 * z0)
    # ngspice's k is 3.5e-7 off CODATA: measured 1.5e-6 dB.
    assert got["nf_db"] == pytest.approx(10 * math.log10(1 + (f_sim - 1) * t_sim / 290.0), abs=1e-5)


# ── rf_compression_ip3 ──

def test_compression_and_ip3_of_a_memoryless_cubic():
    """Matched 50 Ohm input, v(p2) = a1 v + a3 v^3 with v = A cos per tone (A = EMF/2).
    One tone: fund = a1 A + 3/4 a3 A^3. Two tones: fund = a1 A + 9/4 a3 A^3, IM3 = 3/4 |a3| A^3."""
    a1, a3, z0 = 10.0, -40.0, 50.0
    d = ps.Subcircuit("cubic", ["p1", "p2"])
    d.R(name="in", positive="p1", negative=d.gnd, value=z0)
    d.BV(name="o", positive="p2", negative=d.gnd, expression=f"{a1}*v(p1)+{a3}*v(p1)*v(p1)*v(p1)")
    lo = -30.0
    levels = [lo, lo + 10, *np.arange(-8.0, -1.0, 0.5)]
    got = rf_compression_ip3(ps, d, frequency=1e6, levels_dbm=levels).measure()
    amp2 = lambda dbm: 2 * z0 * 1e-3 * 10 ** (dbm / 10)  # A^2 for an available power
    dbm = lambda a2: 10 * math.log10(a2 / (2 * z0) / 1e-3)
    r0 = 1 + 0.75 * a3 / a1 * amp2(lo)
    a2_1db = (1 - r0 * 10 ** (-1 / 20)) / (0.75 * abs(a3 / a1))
    iip3 = lo + 10 * math.log10(abs(a1 + 2.25 * a3 * amp2(lo)) / (0.75 * abs(a3) * amp2(lo)))
    # Samples are interpolated between points 1/(16 fs) = 6.1 ns apart on a 1 MHz carrier: the
    # amplitude falls (w h)^2/12 = 1.2e-4, 1.1e-3 dB (measured 1.5e-3 dB on gain, 6.4e-3 dB on
    # IIP3; both 16x less at 64 steps per sample, so O(h^2)).
    assert got["gain_db"] == pytest.approx(20 * math.log10(abs(r0) * a1), abs=3e-3)
    assert got["iip3_dbm"] == pytest.approx(iip3, abs=0.01)
    assert got["oip3_dbm"] == pytest.approx(iip3 + 20 * math.log10(abs(r0) * a1), abs=0.013)
    assert got["im3_slope"] == pytest.approx(3.0, abs=1e-5)  # IM3 is exactly cubic in A: measured 4e-7
    # Linear interpolation of the gain curve between levels 0.5 dB apart plus the gain offset
    # above: measured 8.8e-4 dB.
    assert got["p1db_in_dbm"] == pytest.approx(dbm(a2_1db), abs=0.01)
