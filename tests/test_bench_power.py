"""Power, reference, bias, current-mode and charge-based benches, run on ngspice.

Oracle for every test: a behavioural DUT whose answer the test computes in
closed form with numpy (partial fractions, charge conservation, line
intersections), never the way the bench's extractor does. The LDO-M
macromodel numbers come from the power/bias research spec (section 2).
"""

import math

import numpy as np
import pytest

import spicerack as ps
from spicerack.testbenches.bias import (
    charge_pump_current,
    current_mirror_output,
    current_reference,
    switched_cap_transfer,
    transconductor_gm,
)
from spicerack.testbenches.design import amplifier_current_gain
from spicerack.testbenches.power import (
    average_power,
    regulator_line_sweep,
    regulator_load_sweep,
    regulator_loop_margins,
    regulator_step,
    supply_rejection,
    supply_startup,
)

# ── LDO-M: Vref -> G error amp (gm into R_ea || C_ea) -> G pass vin->vout, r_ds, divider ──

VREF, R1, R2, GM_EA, R_EA, GP = 0.6, 100e3, 100e3, 100e-6, 10e6, 0.1
BETA, RDIV, A0 = R2 / (R1 + R2), R1 + R2, GM_EA * R_EA


def _ldo(rds, cea=88e-12, *, ron=None, iq0=0.0, fb_pin=False, pole2_hz=None):
    """``ron``: pass is ``min(gp*V(ea), (V(vin)-V(vout))/ron)``. ``fb_pin``: no internal
    divider, the error amp reads pin ``fb``. ``pole2_hz``: an RC pole between ea and the pass."""
    d = ps.Subcircuit("ldo", ["vin", "vout", "fb"] if fb_pin else ["vin", "vout"])
    d.V(name="ref", positive="ref", negative=d.gnd, value=VREF)
    if not fb_pin:
        d.R(name="1", positive="vout", negative="fb", value=R1)
        d.R(name="2", positive="fb", negative=d.gnd, value=R2)
    d.G(name="ea", positive=d.gnd, negative="ea", control_positive="ref", control_negative="fb",
        transconductance=GM_EA)
    d.R(name="ea", positive="ea", negative=d.gnd, value=R_EA)
    d.C(name="ea", positive="ea", negative=d.gnd, value=cea)
    drive = "ea"
    if pole2_hz:
        d.E(name="p2", positive="p2a", negative=d.gnd, control_positive="ea", control_negative=d.gnd,
            voltage_gain=1.0)
        d.R(name="p2", positive="p2a", negative="p2", value=1e3)
        d.C(name="p2", positive="p2", negative=d.gnd, value=1 / (2 * math.pi * 1e3 * pole2_hz))
        drive = "p2"
    if ron is None:
        d.G(name="pass", positive="vin", negative="vout", control_positive=drive, control_negative=d.gnd,
            transconductance=GP)
    else:
        d.BI(name="pass", positive="vin", negative="vout", expression=f"min({GP}*V({drive}),(V(vin)-V(vout))/{ron})")
    d.R(name="ds", positive="vin", negative="vout", value=rds)
    if iq0:
        d.I(name="q", positive="vin", negative=d.gnd, value=iq0)
    return d


def _vout_reg(vin, rds, i_load):
    """Regulated DC V_out of LDO-M: KCL at vout with v_ea = A0 (Vref - beta vout)."""
    return (vin / rds + GP * A0 * VREF - i_load) / (1 / rds + 1 / RDIV + GP * A0 * BETA)


def test_supply_rejection_matches_ldo_m_closed_form():
    rds, cea, cout = 100.0, 88e-12, 1e-9
    bench = supply_rejection(ps, _ldo(rds, cea), supply_node="vin", supply_v=2.5, load_current=1e-3,
                             output_capacitance=cout, spot_hz=(1e3, 1e5, 1e6))
    ac = bench.runner()
    m = bench.metrics(ac)
    f = np.asarray(ac.frequency)
    s = 2j * np.pi * f
    h = (1 / rds) / (1 / rds + 1 / RDIV + s * cout + GP * BETA * A0 / (1 + s * R_EA * cea))
    psrr = -20 * np.log10(np.abs(h))
    # AC is one linear solve: |H| to ~1e-6, i.e. 1e-4 dB; spots sit on grid points (20/dec from 10 Hz).
    assert m["psrr_dc_db"] == pytest.approx(psrr[0], abs=1e-4)
    assert m["psrr_min_db"] == pytest.approx(psrr.min(), abs=1e-4)
    assert m["psrr_min_hz"] == pytest.approx(f[np.argmin(psrr)], rel=1e-9, abs=0)
    for name, fs in (("psrr_db_at_1khz", 1e3), ("psrr_db_at_100khz", 1e5), ("psrr_db_at_1mhz", 1e6)):
        assert m[name] == pytest.approx(psrr[np.argmin(np.abs(f - fs))], abs=1e-4)
    # Spec golden (numpy, recorded in the research note), 3 decimals as published. The bench's
    # "DC" is the lowest swept frequency, 10 Hz, which sits 0.013 dB below the true DC value.
    assert -20 * math.log10((1 / rds) / (1 / rds + 1 / RDIV + GP * BETA * A0)) == pytest.approx(73.981, abs=1e-3)
    assert (m["psrr_db_at_1khz"], m["psrr_db_at_100khz"]) == pytest.approx((58.988, 19.121), abs=1e-3)


def test_regulator_line_sweep_dropout_and_line_regulation():
    rds, ron, iq0, il = 100.0, 2.0, 50e-6, 0.1
    bench = regulator_line_sweep(ps, _ldo(rds, ron=ron, iq0=iq0), vout_nominal=1.2, vin_nominal=2.5, vin_max=3.3,
                                 load_current=il)
    m = bench.measure()
    v_nom = _vout_reg(2.5, rds, il)
    rpar = ron * rds / (ron + rds)
    # Piecewise-linear DUT: every metric sits on a linear segment, so interpolation is exact;
    # 1e-6 covers Newton's stopping point on a 1.2 V node (reltol 1e-3 on the *update*, which is ~0 here).
    assert m["vout_v"] == pytest.approx(v_nom, rel=1e-6, abs=0)
    assert m["line_regulation_v_per_v"] == pytest.approx((1 / rds) / (1 / rds + 1 / RDIV + GP * BETA * A0), rel=1e-6, abs=0)
    assert m["line_regulation_pct_per_v"] == pytest.approx(100 * m["line_regulation_v_per_v"] / v_nom, rel=1e-9, abs=0)
    assert m["dropout_v"] == pytest.approx((il + 1.1 / RDIV) * rpar, rel=1e-6, abs=0)
    assert m["ground_current_a"] == pytest.approx(iq0 + v_nom / RDIV, rel=1e-6, abs=0)
    # Metamorphic: halving the sweep step must not move the dropout point.
    fine = regulator_line_sweep(ps, _ldo(rds, ron=ron, iq0=iq0), vout_nominal=1.2, vin_nominal=2.5, vin_max=3.3,
                                load_current=il, vin_step=2.5e-3).measure()
    assert fine["dropout_v"] == pytest.approx(m["dropout_v"], rel=1e-6, abs=0)


def test_regulator_load_sweep_matches_ldo_m():
    rds, iq0 = 1e3, 50e-6
    m = regulator_load_sweep(ps, _ldo(rds, iq0=iq0), vin_nominal=2.5, i_load_max=10e-3).measure()
    v0, v1 = _vout_reg(2.5, rds, 0.0), _vout_reg(2.5, rds, 10e-3)
    # Linear DUT, DC solve: only solver precision.
    assert m["load_regulation_ohm"] == pytest.approx(1 / (1 / rds + 1 / RDIV + GP * BETA * A0), rel=1e-6, abs=0)
    assert m["load_regulation_pct"] == pytest.approx(100 * (v0 - v1) / v0, rel=1e-6, abs=0)
    assert m["quiescent_current_a"] == pytest.approx(iq0 + v0 / RDIV, rel=1e-6, abs=0)
    assert m["ground_current_max_load_a"] == pytest.approx(iq0 + v1 / RDIV, rel=1e-6, abs=0)


def _ldo_step_response(rds, cea, cout, d_i, t_r, t):
    """V_out deviation for a load ramp of d_i over t_r, by partial fractions.

    H(s) = -(1 + s/w1) / D(s), D = (G_o + sC)(1 + s/w1) + beta gp A0. With
    G(t) the integral of the step response, a ramp gives d_i [G(t) - G(t - t_r)] / t_r.
    """
    w1, g_o = 1 / (R_EA * cea), 1 / rds + 1 / RDIV
    num = np.array([-1 / w1, -1.0])
    den = np.array([cout / w1, cout + g_o / w1, g_o + BETA * GP * A0])
    poles = np.roots(den)
    h0 = num[-1] / den[-1]
    dden = np.polyder(den)

    def ramp_integral(x):
        x = np.maximum(x, 0.0)
        acc = h0 * x + 0j
        for p in poles:
            acc += np.polyval(num, p) / (p * p * np.polyval(dden, p)) * (np.exp(p * x) - 1)
        return acc.real

    return d_i * (ramp_integral(t) - ramp_integral(t - t_r)) / t_r, d_i * h0


def _last_exit(f, band, t):
    """Last time |f(t)| > band, by bisection inside the last dense-grid bracket."""
    out = np.nonzero(np.abs(f(t)) > band)[0][-1]
    lo, hi = t[out], t[out + 1]
    for _ in range(80):
        mid = 0.5 * (lo + hi)
        lo, hi = (mid, hi) if abs(f(np.array([mid]))[0]) > band else (lo, mid)
    return lo


@pytest.mark.parametrize("kind", ["load", "line"])
def test_regulator_step_matches_partial_fractions(kind):
    rds, cea, cout, t_r, hold = 100.0, 88e-12, 1e-9, 10e-9, 5e-6
    d_vin = 0.5
    # A line step of d_vin through r_ds is a load step of -d_vin/r_ds: same D(s), same zero.
    d_i = 1e-3 if kind == "load" else -d_vin / rds
    bench = regulator_step(ps, _ldo(rds, cea), kind=kind, vin_nominal=2.5, vin_low=2.5, vin_high=2.5 + d_vin,
                           i_low=1e-3, i_high=2e-3, t_edge=t_r, delay=1e-6, hold=hold, output_capacitance=cout,
                           max_time=1e-9)
    m = bench.measure()
    t = np.linspace(0, hold, 200001)
    y, final = _ldo_step_response(rds, cea, cout, d_i, t_r, t)
    peak = -y.min() if kind == "load" else y.max()
    v_pre = _vout_reg(2.5, rds, 1e-3)
    t_s = _last_exit(lambda x: _ldo_step_response(rds, cea, cout, d_i, t_r, x)[0] - final, 0.01 * v_pre, t)
    # Ringing at ~0.9 MHz sampled every 1 ns: the peak between samples is off by (w h)^2/8 ~ 4e-6;
    # trapezoidal error is smaller still. 1e-3 leaves room for ngspice's LTE step control.
    key = "undershoot_v" if kind == "load" else "overshoot_v"
    assert m[key] == pytest.approx(peak, rel=1e-3, abs=0)
    assert m["settling_up_s"] == pytest.approx(t_s, rel=1e-3, abs=0)
    assert m["final_shift_v"] == pytest.approx(final, rel=1e-3, abs=0)
    if kind == "load":
        assert peak == pytest.approx(62.667e-3, rel=1e-4, abs=0)  # spec golden: 1 mA in 10 ns


def _clamp_rc(c_in=0.0, r_in=1e3):
    d = ps.Subcircuit("clamp", ["vdd", "vout"])
    d.BV(name="x", positive="x", negative=d.gnd, expression="min(V(vdd),1.2)")
    d.R(name="f", positive="x", negative="vout", value=1e3)
    d.C(name="f", positive="vout", negative=d.gnd, value=1e-9)
    if c_in:
        d.R(name="in", positive="vdd", negative="cin", value=r_in)
        d.C(name="in", positive="cin", negative=d.gnd, value=c_in)
    return d


def test_supply_startup_time_and_inrush_closed_form():
    vdd, ramp, tau, c_in = 1.8, 10e-6, 1e-6, 1e-9
    bench = supply_startup(ps, _clamp_rc(c_in), v_expected=1.2, vdd=vdd, ramp_time=ramp, settle_window=20e-6,
                           max_time=tau / 1000)
    m = bench.measure()
    a = vdd / ramp
    t1 = 1.2 / a
    want = t1 + tau * math.log(a * tau * (1 - math.exp(-t1 / tau)) / 0.012)
    # A ramp into RC: exact exponentials; max step tau/1000 keeps the trapezoid well under 1e-3.
    assert m["started"] == 1.0
    assert m["startup_time_s"] == pytest.approx(want, rel=1e-3, abs=0)
    assert m["startup_after_supply_s"] == pytest.approx(want - ramp, rel=1e-2, abs=0)
    assert m["startup_overshoot_v"] == 0.0
    assert m["inrush_peak_a"] == pytest.approx(c_in * a * (1 - math.exp(-ramp / (1e3 * c_in))), rel=1e-3)


def _bistable(kick=False):
    """x has stable states near 0 and V(vdd): with no kick the circuit never starts."""
    d = ps.Subcircuit("bist", ["vdd", "vout"])
    d.C(name="x", positive="vout", negative=d.gnd, value=1e-9)
    d.BI(name="x", positive=d.gnd, negative="vout",
         expression="1e-3*(V(vdd)*0.5*(1+tanh((V(vout)-0.9)/0.05))-V(vout))")
    if kick:
        d.PieceWiseLinearCurrentSource(name="kick", positive=d.gnd, negative="vout",
                                       values=[(0, 0), (12e-6, 0), (12.01e-6, 5e-3), (13e-6, 5e-3), (13.01e-6, 0)])
    return d


@pytest.mark.parametrize("kick", [False, True])
def test_supply_startup_catches_a_circuit_that_never_starts(kick):
    m = supply_startup(ps, _bistable(kick), v_expected=1.8, vdd=1.8, ramp_time=10e-6, settle_window=30e-6).measure()
    # Boolean oracle: the zero state is stable without the kick, the kick crosses the 0.9 V threshold.
    assert m["started"] == float(kick)
    assert math.isinf(m["startup_time_s"]) != kick


def _pm_gm(t_of_f):
    """PM and GM of T(f) by bisection on a dense log grid (not the bench's interpolation)."""
    f = np.logspace(0, 9, 20001)
    t = t_of_f(f)
    out = {}
    k = np.nonzero(np.diff(np.sign(np.abs(t) - 1)))[0][0]
    lo, hi = f[k], f[k + 1]
    for _ in range(100):
        mid = math.sqrt(lo * hi)
        lo, hi = (mid, hi) if abs(t_of_f(np.array([mid]))[0]) > 1 else (lo, mid)
    out["pm"] = 180 + math.degrees(np.angle(t_of_f(np.array([lo]))[0]))
    # Phase crossover: Im(T) turns from - to + while Re(T) < 0.
    k = np.nonzero((t.real[1:] < 0) & (t.imag[:-1] < 0) & (t.imag[1:] >= 0))[0]
    if len(k):
        lo, hi = f[k[0]], f[k[0] + 1]
        for _ in range(100):
            mid = math.sqrt(lo * hi)
            lo, hi = (mid, hi) if t_of_f(np.array([mid]))[0].imag < 0 else (lo, mid)
        out["gm"] = -20 * math.log10(abs(t_of_f(np.array([lo]))[0]))
    return out


def _loop_t(rds, cea, c, esr, r_l=None, pole2_hz=None):
    def t(f):
        s = 2j * np.pi * f
        y = 1 / rds + 1 / RDIV + (0 if r_l is None else 1 / r_l) + 1 / (esr + 1 / (s * c))
        p2 = 1 if pole2_hz is None else 1 / (1 + s / (2 * np.pi * pole2_hz))
        return BETA * GM_EA * R_EA / (1 + s * R_EA * cea) * GP * p2 / y
    return t


def test_regulator_loop_margins_esr_grid():
    rds, cea, c = 1e3, 15.9e-12, 1e-6
    esrs = (0.0, 0.1, 0.3, 1.0)
    bench = regulator_loop_margins(ps, _ldo(rds, cea, fb_pin=True), vin_nominal=2.5, vout_nominal=1.2,
                                   divider=(R1, R2), cout_esr=[(c, e) for e in esrs], stop_frequency=1e8)
    m = bench.measure()
    pms = [_pm_gm(_loop_t(rds, cea, c, e))["pm"] for e in esrs]
    assert pms == pytest.approx([0.75, 3.96, 10.35, 31.91], abs=0.01)  # spec golden
    # Tian is exact; 50 pts/dec, ln-interpolation: (dlnf)^2/8 ~ 2.7e-4 rad of phase error, ~0.02 deg.
    assert m["phase_margin_deg_min"] == pytest.approx(min(pms), abs=0.05)
    assert m["worst_esr_ohm"] == 0.0
    assert math.isinf(m["gain_margin_db_min"])  # two poles and a zero never reach -180 deg


def test_regulator_loop_margins_load_grid_with_gain_margin():
    rds, cea, c, esr, p2 = 100.0, 88e-12, 1e-9, 0.0, 2e6
    loads = (0.0, 10e-3)
    bench = regulator_loop_margins(ps, _ldo(rds, cea, fb_pin=True, pole2_hz=p2), vin_nominal=2.5,
                                   vout_nominal=1.2, divider=(R1, R2), load_currents=loads, cout_esr=[(c, esr)],
                                   stop_frequency=1e8)
    m = bench.measure()
    rows = [_pm_gm(_loop_t(rds, cea, c, esr, 1.2 / il if il else None, p2)) for il in loads]
    worst = int(np.argmin([r["pm"] for r in rows]))
    assert m["phase_margin_deg_min"] == pytest.approx(rows[worst]["pm"], abs=0.05)  # as above
    assert m["worst_load_a"] == loads[worst]
    # Same ln-f interpolation bound on |T|: (dlnf)^2/8 * |d2 ln|T|| < 1e-3 nepers ~ 0.01 dB.
    assert m["gain_margin_db_min"] == pytest.approx(min(r["gm"] for r in rows), abs=0.02)


def test_average_power_of_switched_cap_plus_resistor():
    vdd, c, r, f = 1.8, 1e-12, 100e3, 1e6
    d = ps.Subcircuit("swc", ["vdd", "clk"])
    d.raw_spice(".model swp SW(ron=1k roff=1e12 vt=0.5)")
    d.S(name="chg", positive="vdd", negative="x", control_positive="clk", control_negative=d.gnd, model="swp")
    d.raw_spice(".model swn SW(ron=1k roff=1e12 vt=-0.5)")
    d.S(name="dis", positive="x", negative=d.gnd, control_positive=d.gnd, control_negative="clk", model="swn")
    d.C(name="x", positive="x", negative=d.gnd, value=c)
    d.R(name="r", positive="vdd", negative=d.gnd, value=r)
    tb = ps.Testbench(d)
    tb.V(name="dd", positive="vdd", negative="0", value=vdd)
    tb.PulseVoltageSource(name="clk", positive="clk", negative="0", initial_value=0.0, pulsed_value=1.0,
                          pulse_width=0.5 / f, period=1 / f, rise_time=1e-9, fall_time=1e-9)
    tran = tb.transient(step_time=1e-9, end_time=13 / f, max_time=1e-9)
    p = average_power(tran, "Vdd", 2.25 / f, 12.25 / f, node="vdd")
    # Settled (RC = 1 ns << T/2): each cycle draws C*V^2 from the supply, exactly, plus V^2/R.
    # Trapezoid of i equals the trapezoidal integrator's charge, so only roff leak (~1e-6) remains.
    assert p == pytest.approx(c * vdd ** 2 * f + vdd ** 2 / r, rel=1e-4, abs=0)


# ── current mode ──

N, I_IN, LAM, R_TRI, R_D = 2.0, 10e-6, 0.005, 10e3, 10e3


def _mirror(c_in=0.0, i_th=0.0):
    """I_out = min(V/R_tri, N max(I_in - i_th, 0) (1 + lam V)) into the output pin; R_D is the
    diode. ``i_th`` > 0 turns the mirror off near zero input, as a MOS mirror is."""
    d = ps.Subcircuit("mir", ["iin", "iout"])
    d.V(name="sense", positive="iin", negative="a", value=0.0)
    d.R(name="d", positive="a", negative=d.gnd, value=R_D)
    if c_in:
        d.C(name="in", positive="iin", negative=d.gnd, value=c_in)
    d.BI(name="m", positive="iout", negative=d.gnd,
         expression=f"min(V(iout)/{R_TRI},{N}*max(i(Vsense)-{i_th},0)*(1+{LAM}*V(iout)))")
    return d


def test_current_mirror_output_ratio_rout_compliance():
    # 1 mV steps put the 1 % point (0.1989 V) and the knee (0.2002 V) in different intervals.
    vnom, step = 0.9, 1e-3
    m = current_mirror_output(ps, _mirror(), input_current=I_IN, nominal_ratio=N, vout_nominal=vnom,
                              vout_step=step).measure()
    i_nom = N * I_IN * (1 + LAM * vnom)
    knee = N * I_IN * R_TRI / (1 - N * I_IN * R_TRI * LAM)
    # Piecewise-linear DUT: central differences and interpolation are exact on each segment.
    assert m["iout_a"] == pytest.approx(i_nom, rel=1e-6, abs=0)
    assert m["ratio_error"] == pytest.approx(LAM * vnom, rel=1e-6, abs=0)
    assert m["rout_ohm"] == pytest.approx(1 / (N * I_IN * LAM), rel=1e-6, abs=0)
    assert 0.99 * i_nom * R_TRI < knee  # the 1 % point lies on the triode line
    assert m["compliance_v"] == pytest.approx(0.99 * i_nom * R_TRI, rel=1e-6, abs=0)
    # r_out jumps at the knee; central differences straddle it one step each side.
    assert abs(m["compliance_rout_v"] - knee) < 2 * step
    assert m["vin_v"] == pytest.approx(I_IN * R_D, rel=1e-6, abs=0)


def test_current_gain_reads_a_biased_mirror():
    fc = 10e3
    c_in = 1 / (2 * math.pi * R_D * fc)
    # Unbiased (the old defaults), this mirror is off and its output in triode: gain 0.
    bench = amplifier_current_gain(ps, _mirror(c_in, i_th=1e-6), input_bias_current=I_IN, output_bias_voltage=0.9)
    m = bench.metrics(bench.testbench.ac(variation="dec", number_of_points=80, start_frequency=1.0,
                                         stop_frequency=100e6))
    # Small-signal dI_out/dI_in in saturation, through the R_D || C_in pole at fc.
    assert m["current_gain_midband"] == pytest.approx(N * (1 + LAM * 0.9), rel=1e-6, abs=0)
    # Log-x interpolation of a one-pole dB curve at 80/dec: (1/80)^2/8 * 2.3 dec ~ 5e-5 dec ~ 1e-4.
    assert m["current_gain_bandwidth_hz"] == pytest.approx(fc, rel=2e-4, abs=0)


def test_current_reference_ptat_tempco_and_intercept():
    d = ps.Subcircuit("ptat", ["vdd", "iref"])
    d.BI(name="p", positive=d.gnd, negative="iref", expression="1e-6*(temper+273.15)/300.15")
    m = current_reference(ps, d).measure()
    # Pure PTAT pushed out of the pin: exact line, exact box tempco 1e6/300.15 ppm/C.
    assert m["iref_27c_a"] == pytest.approx(-1e-6, rel=1e-9, abs=0)
    assert m["tempco_box_ppm_c"] == pytest.approx(1e6 / 300.15, rel=1e-6, abs=0)
    assert m["ptat_intercept_c"] == pytest.approx(-273.15, abs=1e-6)


def test_current_reference_supply_sensitivity():
    i0, ro, vb = 10e-6, 1e6, 0.9
    d = ps.Subcircuit("ro", ["vdd", "iref"])
    d.I(name="0", positive="vdd", negative="iref", value=i0)
    d.R(name="o", positive="vdd", negative="iref", value=ro)
    m = current_reference(ps, d, sweep="supply", supply=1.8, supply_start=1.2, supply_stop=3.0,
                          output_bias_v=vb).measure()
    i_ref = -(i0 + (1.8 - vb) / ro)
    # Linear DUT: dI/dV = -1/r_o exactly.
    assert m["iref_a"] == pytest.approx(i_ref, rel=1e-6, abs=0)
    assert m["supply_sensitivity_pct_per_v"] == pytest.approx(100 * (-1 / ro) / i_ref, rel=1e-6, abs=0)


def test_transconductor_gm_of_a_tanh_pair():
    it, vs, vos = 100e-6, 0.2, 0.0123
    d = ps.Subcircuit("ota", ["inp", "inn", "out"])
    d.BI(name="g", positive=d.gnd, negative="out", expression=f"{it}*tanh((V(inp)-V(inn)-{vos})/{vs})")
    m = transconductor_gm(ps, d).measure()
    # tanh: G_m0 = I_t/V_s; 1 % range = acosh(1/sqrt(0.99)) V_s; I is odd about V_os so its
    # zero interpolates to ~h^3. Central difference at h = 1 mV: (h/V_s)^2/3 ~ 1e-5 on G_m0;
    # linear interpolation of G_m to its 0.99 crossing: h^2/8 |g''/g'| ~ 6e-6 V, 3e-4 of the range.
    assert m["offset_v"] == pytest.approx(vos, abs=1e-7)
    assert m["gm0_s"] == pytest.approx(it / vs, rel=1e-4, abs=0)
    assert m["linear_range_v"] == pytest.approx(math.acosh(1 / math.sqrt(0.99)) * vs, rel=1e-3, abs=0)
    assert m["iout_max_a"] == pytest.approx(it * math.tanh((0.5 + vos) / vs), rel=1e-9, abs=0)


# ── charge based ──

def _sc_integrator(c1, c2, a0, tau):
    """Parasitic-insensitive integrator, S switches, single-pole op amp gm R || C buffered."""
    d = ps.Subcircuit("sci", ["vin", "vout", "phi1", "phi2"])
    d.raw_spice(".model sws SW(ron=1 roff=1e12 vt=0.5)")
    for name, p, n, clk in (("1", "vin", "l", "phi1"), ("2", "l", "0", "phi2"),
                            ("3", "r", "0", "phi1"), ("4", "r", "x", "phi2")):
        d.S(name=name, positive=p, negative=n, control_positive=clk, control_negative=d.gnd, model="sws")
    d.C(name="1", positive="l", negative="r", value=c1)
    d.C(name="2", positive="x", negative="vout", value=c2)
    gm, rp = 1e-3, a0 / 1e-3
    beta = c2 / (c1 + c2)
    d.G(name="a", positive="p", negative=d.gnd, control_positive="x", control_negative=d.gnd, transconductance=gm)
    d.R(name="p", positive="p", negative=d.gnd, value=rp)
    d.C(name="p", positive="p", negative=d.gnd, value=tau * (1 + a0 * beta) / rp)
    d.E(name="b", positive="vout", negative=d.gnd, control_positive="p", control_negative=d.gnd, voltage_gain=1.0)
    return d


def test_switched_cap_integrator_charge_conservation():
    c1, c2, a0, tau, t_clk = 1e-12, 4e-12, 1000.0, 20e-9, 1e-6
    m = switched_cap_transfer(ps, _sc_integrator(c1, c2, a0, tau), nominal_gain=c1 / c2, t_clk=t_clk).measure()
    # Charge at the virtual ground across one phi1/phi2 cycle with V_x = -V_o/A.
    d = 1 + (1 + c1 / c2) / a0
    # Linear caps conserve charge exactly under the trapezoidal rule; roff = 1e12 leaks ~1e-7 per cycle.
    assert m["charge_gain"] == pytest.approx((c1 / c2) / d, rel=1e-4, abs=0)
    assert m["leak_factor"] == pytest.approx((1 + 1 / a0) / d, rel=1e-4, abs=0)
    assert m["gain_error"] == pytest.approx(1 / d - 1, abs=1e-4)  # the charge_gain tolerance, in absolute
    # First-order closed loop from the switch closing (half the 1 ns clock edge, vt = 0.5)
    # for tau ln(100); the switch fires on the first time point past vt: one 1 ns step.
    assert m["settling_time_s"] == pytest.approx(0.5e-9 + tau * math.log(100), abs=1e-9)


def test_charge_pump_currents_mismatch_and_range():
    i0u, i0d, rup, rdn, rt = 100e-6, 98e-6, 1e6, 500e3, 4e3
    d = ps.Subcircuit("cp", ["cp", "up", "dn"])
    d.BI(name="u", positive=d.gnd, negative="cp",
         expression=f"u(V(up)-0.5)*min({i0u}+(0.9-V(cp))/{rup},(1.8-V(cp))/{rt})")
    d.BI(name="d", positive="cp", negative=d.gnd,
         expression=f"u(V(dn)-0.5)*min({i0d}+(V(cp)-0.9)/{rdn},V(cp)/{rt})")
    m = charge_pump_current(ps, d).measure()

    def i_up(v):
        return np.minimum(i0u + (0.9 - v) / rup, (1.8 - v) / rt)

    def i_dn(v):
        return np.minimum(i0d + (v - 0.9) / rdn, v / rt)

    # The 5 % edges lie on the triode lines (the kinks sit a grid interval away): solve them.
    left, right = 0.95 * i_dn(0.9) * rt, 1.8 - 0.95 * i_up(0.9) * rt
    assert i_dn(left) == pytest.approx(0.95 * i_dn(0.9)) and i_up(right) == pytest.approx(0.95 * i_up(0.9))
    v = np.linspace(left, right, 100001)
    mis = 200 * np.abs(i_up(v) - i_dn(v)) / (i_up(v) + i_dn(v))
    # Piecewise linear: interpolation is exact; the mismatch max sits at a window edge.
    assert m["i_up_a"] == pytest.approx(i0u, rel=1e-6, abs=0)
    assert m["i_dn_a"] == pytest.approx(i0d, rel=1e-6, abs=0)
    assert m["output_range_v"] == pytest.approx(right - left, rel=1e-6, abs=0)
    assert m["mismatch_pct_max"] == pytest.approx(mis.max(), rel=1e-6, abs=0)
    assert m["both_on_net_a_at_mid"] == pytest.approx(i0u - i0d, rel=1e-6, abs=0)
