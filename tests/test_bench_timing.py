"""Timing benches against behavioural DUTs whose answer is known in closed form.

Every expected value is computed here independently of the extractor: a closed form,
the trapezoidal dispersion relation, the linear phase-domain PLL model, a nodal
solve, or a numpy Monte Carlo of the estimator. ngspice runs every deck; a missing
simulator fails.
"""
import math

import numpy as np
import pytest

import spicerack as ps
from spicerack.testbenches.analysis import crossings
from spicerack.testbenches.timing import (
    clock_buffer_timing,
    crystal_negative_resistance,
    delay_line_range,
    oscillator_frequency,
    oscillator_jitter,
    oscillator_sweep,
)
from spicerack.testbenches.timing_2 import (
    dll_lock,
    pfd_cp_transfer,
    pll_lock_time,
    pll_loop_margins,
    pll_reference_spur,
)

TAU = 2 * math.pi


# ── behavioural DUTs ──

def _vco(name, f0, k=0.0, k2=0.0, ctl_value=None, extra=()):
    """Integrating VCO: f = f0 + k*V(ctl) + k2*V(ctl)**2 exactly, since the trapezoidal
    rule integrates a constant current into 1 F exactly. Output sin(2*pi*phase).

    At reltol=1e-3 Newton stops after one linearised step and sin() reads up to
    (2*pi*f*h)**2/2 = 5e-4 high at 200 points/period; reltol=1e-6 bounds that at 1e-6."""
    d = ps.Subcircuit(name, ["ctl", "out"])
    d.BI(name="ph", positive="0", negative="ph", expression=f"{f0}+{k}*V(ctl)+{k2}*V(ctl)*V(ctl)")
    d.C(name="ph", positive="ph", negative="0", value=1.0)
    d.R(name="ph", positive="ph", negative="0", value=1e15)
    d.BV(name="out", positive="out", negative="0", expression=f"sin({TAU}*V(ph))")
    d.raw_spice(".ic v(ph)=0")
    d.raw_spice(".options reltol=1e-6")
    if ctl_value is not None:
        d.V(name="ctl", positive="ctl", negative="0", value=ctl_value)
    for line in extra:
        d.raw_spice(line)
    return d


def _pfd(d, ref, fb, t_rst=1e-9, dac_rise=1e-12, dn_ic=0):
    """XSPICE tri-state PFD: two D flip-flops reset through an AND with ``t_rst`` delay."""
    for line in [f"aadc [{ref} {fb}] [refd fbd] adc1", ".model adc1 adc_bridge(in_low=0.5 in_high=0.5)",
                 "apu one pu1", ".model pu1 d_pullup",
                 "aff1 one refd null rst up null dff1", "aff2 one fbd null rst dn null dff2",
                 ".model dff1 d_dff(clk_delay=1e-12 reset_delay=1e-12 rise_delay=1e-12 fall_delay=1e-12)",
                 f".model dff2 d_dff(clk_delay=1e-12 reset_delay=1e-12 rise_delay=1e-12 fall_delay=1e-12 ic={dn_ic})",
                 "aand [up dn] rst and1", f".model and1 d_and(rise_delay={t_rst} fall_delay=1e-12)",
                 "adac [up dn] [upa dna] dac1",
                 f".model dac1 dac_bridge(out_low=0 out_high=1 t_rise={dac_rise} t_fall=1e-12)"]:
        d.raw_spice(line)


# Charge-pump PLL (N = 1): Ip, Kv, R, C1, C2. wn = sqrt(Ip*Kv/C1) = 2*pi*250 kHz, zeta = 0.706.
IP, KV, R1, C1, C2 = 1e-4, 2e6, 11.1e3, 81e-12, 1.62e-12


def _pll(f_ref, n=1, leak=0.0):
    """VCO at n*f_ref + n*KV*V(vc); the divider is in the phase domain (fb = phase/n),
    so the loop is the same for every n."""
    d = ps.Subcircuit("pll", ["ref", "vco", "vc"])
    d.BI(name="ph", positive="0", negative="ph", expression=f"{n * f_ref}+{n * KV}*V(vc)")
    d.C(name="ph", positive="ph", negative="0", value=1.0)
    d.R(name="ph", positive="ph", negative="0", value=1e15)
    d.BV(name="vco", positive="vco", negative="0", expression=f"0.5+0.5*tanh(20*sin({TAU}*V(ph)))")
    d.BV(name="fb", positive="fb", negative="0", expression=f"0.5+0.5*tanh(20*sin({TAU / n}*V(ph)))")
    _pfd(d, "ref", "fb")
    d.BI(name="cp", positive="0", negative="vc", expression=f"{IP}*(V(upa)-V(dna))")
    d.R(name="1", positive="vc", negative="mid", value=R1)
    d.C(name="1", positive="mid", negative="0", value=C1)
    d.C(name="2", positive="vc", negative="0", value=C2)
    if leak:
        d.I(name="leak", positive="vc", negative="0", value=leak)
    d.raw_spice(f".ic v(ph)={0.25 * n} v(vc)=0 v(mid)=0")
    return d


def _delay_cell(d, i, inp, out, charge, threshold, cc=1e-12):
    """Ramp delay cell: an input rising edge opens a ~2 ns reset window (input above
    0.5, its 3 ns RC copy still below); the cap then charges at ``charge`` amps and the
    output rises when it crosses ``threshold``. The reset time constant cc/5m = 200 ps
    stays above 2x the 100 ps max step, or the trapezoidal rule rings through the
    threshold; the window holds > 10 of them. The output edge lasts 2/(500*slope),
    0.25 ns at the slowest ramp: sharp, so the next cell's RC window barely depends on
    it (a 2.5 ns edge shifted that window by 0.9 % of kd), but still 5 steps wide."""
    d.R(name=f"rc{i}", positive=inp, negative=f"inrc{i}", value=3e3)
    d.C(name=f"rc{i}", positive=f"inrc{i}", negative="0", value=1e-12)
    d.BI(name=f"c{i}", positive="0", negative=f"c{i}",
         expression=f"(V({inp})>0.5 && V(inrc{i})<0.5) ? -5e-3*V(c{i}) : {charge}")
    d.C(name=f"c{i}", positive=f"c{i}", negative="0", value=cc)
    d.BV(name=f"o{i}", positive=out, negative="0", expression=f"0.5*(1+tanh(500*(V(c{i})-{threshold})))")


def _delay_line(name="vcdl", charge="2e-5", threshold="V(ctl)"):
    """Two ramp cells in series, each under one period. Default: threshold V(ctl), so
    delay = const + 2*(cc/i0)*V(ctl), kd = 100 ns/V. Cell 2 starts far below
    threshold so it cannot fire before its first input edge."""
    d = ps.Subcircuit(name, ["in", "out", "ctl"])
    _delay_cell(d, 1, "in", "mid", charge, threshold)
    _delay_cell(d, 2, "mid", "out", charge, threshold)
    d.raw_spice(".ic v(c1)=0 v(c2)=-10")
    return d


# ── oscillator_frequency ──

def test_lc_van_der_pol_frequency_amplitude_startup():
    """vdP tank: L=1u, C=1n, R=1k, i = -g1*v + g3*v**3. eps = (g1-1/R)*sqrt(L/C) = 0.0063."""
    ind, cap, res, g1, g3 = 1e-6, 1e-9, 1e3, 1.2e-3, 8e-4 / 3
    f0 = 1 / (TAU * math.sqrt(ind * cap))
    d = ps.Subcircuit("vdp", ["t"])
    d.L(name="1", positive="t", negative="0", value=ind)
    d.C(name="1", positive="t", negative="0", value=cap)
    d.R(name="1", positive="t", negative="0", value=res)
    d.BI(name="nl", positive="t", negative="0", expression=f"-{g1}*V(t)+{g3}*V(t)*V(t)*V(t)")
    q = 1e-11  # 10 mV on 1 nF
    bench = oscillator_frequency(ps, d, expected_frequency=f0, output_node="t", kick_node="t",
                                 kick_charge=q, cycles=800, settle_fraction=0.6)
    m = bench.measure()

    # Trapezoidal dispersion at the bench's step h = T/200: w_d = (2/h)*atan(w*h/2).
    h = 1 / (200 * f0)
    f_trap = (2 / h) * math.atan(math.pi * f0 * h) / TAU
    # vdP's own shift is eps**2/16 = 2.5 ppm; 10 ppm allows that plus edge interpolation.
    assert m["frequency_hz"] == pytest.approx(f_trap, rel=10e-6, abs=0)
    assert abs(m["frequency_hz"] / f0 - 1) > 50e-6, "the step must be coarse enough to show dispersion"
    a_inf = math.sqrt(4 * (g1 - 1 / res) / (3 * g3))
    # pp = 2A to O(eps**2); raw-sample extremes read low by (pi/100)**2/8 = 1.2e-4.
    assert m["amplitude_pp_v"] == pytest.approx(2 * a_inf, rel=1e-3, abs=0)
    assert m["duty_cycle"] == pytest.approx(0.5, abs=1e-4)  # odd nonlinearity: symmetric
    # Averaging method: A(t)**2 = A**2/(1+(A**2/A0**2-1)exp(-2*s*t)), s = (g1-1/R)/(2C).
    s, a0 = (g1 - 1 / res) / (2 * cap), q / cap
    t90 = math.log((a_inf ** 2 / a0 ** 2 - 1) / (1 / 0.81 - 1)) / (2 * s)
    # O(eps) averaging error (0.6 %) plus one period of edge quantisation.
    assert m["startup_time_s"] == pytest.approx(t90, rel=0.01, abs=1 / f0)


def test_vco_threshold_sets_duty_and_frequency_is_exact():
    """sin(2*pi*f*t) crossed at 0.5 is high for 1/2 - asin(0.5)/pi = 1/3 of a period."""
    f0 = 10e6
    d = _vco("v", f0, ctl_value=0.0, extra=["Rload vdd 0 1k"])
    bench = oscillator_frequency(ps, d, expected_frequency=f0, threshold=0.5, cycles=100,
                                 supply_node="vdd", vdd=1.8, supply_ramp=1e-6)
    m = bench.measure()
    assert m["supply_current_a"] == pytest.approx(1.8e-3, rel=1e-9, abs=0)  # Ohm's law; sign: drawn from the supply
    assert m["startup_time_s"] == 0.0  # the phase integrator ignores the supply: running before the ramp ends
    assert m["frequency_hz"] == pytest.approx(f0, rel=1e-6, abs=0)  # exact integration; O(h^2) crossing error averages out
    # Linear interpolation at sin = 0.5 errs by <= h**2*|v''|/(8|v'|) = 1.1e-5 T per edge, opposite on the two edges.
    assert m["duty_cycle"] == pytest.approx(1 / 3, abs=2.5e-5)
    assert m["period_spread_s"] < 3e-5 / f0
    assert m["amplitude_pp_v"] == pytest.approx(2.0, rel=2e-4, abs=0)


def test_latched_oscillator_raises_instead_of_zero_hz():
    d = ps.Subcircuit("dead", ["out"])
    d.R(name="1", positive="out", negative="0", value=1e3)
    bench = oscillator_frequency(ps, d, expected_frequency=1e6, supply_node="vdd", cycles=20)
    with pytest.raises(ValueError, match="no oscillation"):
        bench.measure()


# ── oscillator_sweep ──

def test_sweep_recovers_quadratic_tuning_curve():
    f0, k, k2 = 10e6, 2e6, 1e6
    values = [0.0, 0.25, 0.5, 0.75, 1.0]
    bench = oscillator_sweep(ps, _vco("v", f0, k, k2), values=values, sweep_node="ctl",
                             frequency_range=(10e6, 13e6), settle_cycles=20, measure_cycles=50)
    m = bench.measure()
    f = [f0 + k * v + k2 * v * v for v in values]
    for i, fi in enumerate(f):
        assert m[f"frequency_hz_{i}"] == pytest.approx(fi, rel=1e-6, abs=0)  # exact integration, see _vco
    assert m["df_dv_hz_per_v"] == pytest.approx(k + 2 * k2 * 0.5, rel=1e-5, abs=0)  # central difference of a parabola is exact
    chords = [k + k2 * (a + b) for a, b in zip(values, values[1:])]
    assert m["df_dv_max_over_min"] == pytest.approx(max(chords) / min(chords), rel=1e-5, abs=0)
    assert m["tuning_range_ratio"] == pytest.approx(2 * (f[-1] - f[0]) / (f[-1] + f[0]), rel=1e-5, abs=0)
    assert m["monotonic"] == 1.0


def test_sweep_step_that_stops_oscillating_is_named():
    bench = oscillator_sweep(ps, _vco("v", 0.0, 1e7), values=[0.0, 0.5, 1.0], sweep_node="ctl",
                             frequency_range=(5e6, 10e6), settle_cycles=5, measure_cycles=10)
    with pytest.raises(ValueError, match="ctl = 0 V"):
        bench.measure()


# ── oscillator_jitter ──

def _white_fm_estimator_sd(sigma_t, period, n, acc_max, trials=300):
    """Monte Carlo (numpy, no simulator) of the growth-exponent estimator's spread."""
    rng = np.random.default_rng(0)
    m = 2 ** np.arange(int(math.log2(acc_max)) + 1)
    out = []
    for _ in range(trials):
        e = np.cumsum(period + sigma_t * rng.standard_normal(n))
        acc = [np.std(e[k:] - e[:-k], ddof=1) for k in m]
        out.append(np.polyfit(np.log(m), np.log(acc), 1)[0])
    return float(np.std(out))


def test_white_fm_jitter_matches_injected_noise():
    """ctl = trnoise(NA, NT): integrated over T it has variance NA**2*NT*T, so
    sigma_T = (K/f0)*NA*sqrt(NT*T) = 100 ps and L(df) = K**2*NA**2*NT/df**2."""
    f0, k, na, nt, n = 10e6, 1e6, 0.1, 1e-9, 2000
    d = _vco("nv", f0, k, extra=[f"Vctl ctl 0 dc 0 trnoise({na} {nt} 0 0)"])
    bench = oscillator_jitter(ps, d, expected_frequency=f0, cycles=n)
    m = bench.measure()
    sigma_t = (k / f0) * na * math.sqrt(nt / f0)
    # ngspice trnoise ignores every seed (.options seed, rndseed, setseed: checked on
    # ngspice-45), so these are statistical: 4 standard errors, P(false fail) ~ 1e-4 each.
    se_std = 1 / math.sqrt(2 * (n - 1))
    assert m["period_jitter_rms_s"] == pytest.approx(sigma_t, rel=4 * se_std, abs=0)
    # Adjacent period differences are MA(1) with rho = -1/2: var of their sample variance is 3/n.
    assert m["cycle_to_cycle_jitter_rms_s"] == pytest.approx(math.sqrt(2) * sigma_t, rel=4 * 0.5 * math.sqrt(3 / n), abs=0)
    assert m["c_s"] == pytest.approx((k / f0) ** 2 * na ** 2 * nt, rel=4 * math.sqrt(2 / n), abs=0)
    l_db = 10 * math.log10(k ** 2 * na ** 2 * nt / 1e6 ** 2)
    assert m["phase_noise_dbc_hz_at_1mhz"] == pytest.approx(l_db, abs=10 * math.log10(1 + 4 * math.sqrt(2 / n)))
    sd = _white_fm_estimator_sd(sigma_t, 1 / f0, n, 16)
    assert m["jitter_growth_exponent"] == pytest.approx(0.5, abs=4 * sd)
    # Phase random walk: the mean frequency is off by sigma_T/(T*sqrt(n)).
    assert m["frequency_hz"] == pytest.approx(f0, rel=4 * sigma_t * f0 / math.sqrt(n), abs=0)


def test_jitter_refuses_a_noiseless_deck():
    with pytest.raises(ValueError, match="trnoise"):
        oscillator_jitter(ps, _vco("q", 10e6, ctl_value=0.0), expected_frequency=10e6)


# ── clock_buffer_timing ──

def _ramp_buffer(a):
    """Input delayed TD = 1 ns by a matched line, then a comparator at a*V(vdd) whose
    output swings 0..V(vdd): it crosses V(vdd)/2 exactly where V(in) = a*V(vdd)."""
    d = ps.Subcircuit("buf", ["in", "out", "vdd"])
    d.T(name="1", input_positive="in", input_negative="0", output_positive="ind", output_negative="0",
        Z0=50, TD=1e-9)
    d.R(name="term", positive="ind", negative="0", value=50)
    d.BV(name="cmp", positive="out", negative="0", expression=f"V(vdd)/2*(1+tanh(20*(V(ind)-{a}*V(vdd))))")
    return d


def test_buffer_delay_and_duty_distortion_from_ramp_geometry():
    a, td, edge, f = 0.6, 1e-9, 1e-9, 100e6
    bench = clock_buffer_timing(ps, _ramp_buffer(a), vdd=1.0, frequency=f, input_slew=0.8 * edge, cycles=40)
    m = bench.measure()
    # The input ramp crosses a*vdd at a*edge, its 50 % point at edge/2.
    tol = 0.2e-12  # max step T/1000 = 10 ps across a 100 ps-wide tanh: O(h^3) interpolation error
    assert m["tplh_s"] == pytest.approx(td + (a - 0.5) * edge, abs=tol)
    assert m["tphl_s"] == pytest.approx(td + (0.5 - a) * edge, abs=tol)
    assert m["dcd_s"] == pytest.approx(-2 * (a - 0.5) * edge, abs=2 * tol)
    assert m["duty_out"] == pytest.approx(0.5 - 2 * (a - 0.5) * edge * f, abs=2 * tol * f)


def test_buffer_psij_is_the_threshold_supply_sensitivity():
    a, edge, f, rip = 0.6, 1e-9, 100e6, 0.05
    bench = clock_buffer_timing(ps, _ramp_buffer(a), vdd=1.0, frequency=f, input_slew=0.8 * edge,
                                cycles=80, ripple_v=rip)
    m = bench.measure()
    kd = a * edge  # rising delay = TD + (a*V(vdd) - 0.5)*edge: d(delay)/dV(vdd) = a*edge
    assert m["psij_s_per_v"] == pytest.approx(kd, rel=1e-3, abs=0)
    # Ripple f/7.3 = f*10/73: edges sample it at multiples of 2*pi/73, so the extremes
    # come within pi/73 of the peaks.
    assert 2 * kd * rip * math.cos(math.pi / 73) - 0.5e-12 <= m["psij_pp_s"] <= 2 * kd * rip + 0.5e-12


# ── delay_line_range ──

def test_delay_line_kd_and_period_coverage():
    """Current-controlled cells (I = g*V(ctl), fixed 0.3 V threshold): delay =
    const + 2*cc*0.3/(g*V). A threshold-controlled cell would re-fire when the control
    steps up past a cap that already crossed; this one cannot glitch."""
    f, g, vth, cc = 20e6, 2e-5, 0.3, 1e-12
    values = [0.4, 0.5, 0.6, 0.7, 0.8, 0.9]
    bench = delay_line_range(ps, _delay_line(charge=f"{g}*V(ctl)", threshold=vth), values=values, frequency=f,
                             control_node="ctl", settle_cycles=5, measure_cycles=10)
    m = bench.measure()
    k = 2 * cc * vth / g
    h = 1 / (1000 * f)
    # Central difference of k/v over 0.6 and 0.7 is -k/(0.6*0.7) exactly. Each cell's
    # charge starts within one step of its ideal instant: <= h per edge, over 0.1 V.
    assert m["kd_s_per_v"] == pytest.approx(-k / (0.6 * 0.7), abs=2 * h / 0.1)
    d = np.array([m[f"delay_s_{i}"] for i in range(len(values))])
    x = 1 / np.array(values)
    assert np.abs(d - np.polyval(np.polyfit(x, d, 1), x)).max() < h  # const + k/v to the event error
    assert np.polyfit(x, d, 1)[0] == pytest.approx(k, abs=2 * h / (x[0] - x[-1]))
    assert m["delay_min_s"] < 1 / f < m["delay_max_s"]
    assert m["covers_period"] == 1.0
    assert m["harmonic_lock_risk"] == 0.0


# ── crystal_negative_resistance ──

def test_pierce_core_negative_resistance_closed_form():
    """Ideal gm from a to b, C1 at a, C2 at b: Zc = (C1+C2)/(jw C1 C2) - gm/(w**2 C1 C2)."""
    gm, ca, cb, f = 1e-3, 20e-12, 20e-12, 25e6
    d = ps.Subcircuit("pierce", ["xa", "xb"])
    d.G(name="m", positive="xb", negative="0", control_positive="xa", control_negative="0", transconductance=gm)
    d.C(name="a", positive="xa", negative="0", value=ca)
    d.C(name="b", positive="xb", negative="0", value=cb)
    # DC paths; at 25 MHz they move Re(Zc) by ~1e-7 ohm.
    d.R(name="a", positive="xa", negative="0", value=1e12)
    d.R(name="b", positive="xb", negative="0", value=1e12)
    bench = crystal_negative_resistance(ps, d, frequency=f, motional_resistance=25.0)
    m = bench.measure()
    w = TAU * f
    rn = gm / (w ** 2 * ca * cb)
    assert m["negative_resistance_ohm"] == pytest.approx(rn, rel=1e-6, abs=0)  # AC is exact to roundoff
    assert m["negative_resistance_margin"] == pytest.approx(rn / 25.0, rel=1e-6, abs=0)
    assert m["load_capacitance_f"] == pytest.approx(ca * cb / (ca + cb), rel=1e-6, abs=0)


# ── pll_lock_time ──

def _linear_pll(df, t, f_ref):
    """Linear CP-PLL with C2 (states: V(mid), V(vc), phase error in cycles) after a
    reference step df: frequency error Kv*V(vc) - df and phase error in seconds."""
    a = np.array([[-1 / (R1 * C1), 1 / (R1 * C1), 0.0],
                  [1 / (R1 * C2), -1 / (R1 * C2), IP / C2],
                  [0.0, -KV, 0.0]])
    x_ss = -np.linalg.solve(a, np.array([0.0, 0.0, df]))
    lam, vec = np.linalg.eig(a)
    c = np.linalg.solve(vec, -x_ss)
    x = (x_ss[:, None] + vec @ (c[:, None] * np.exp(np.outer(lam, t)))).real
    return KV * x[1] - df, x[2] / f_ref


def test_pll_lock_time_converges_to_the_linear_model():
    """The continuous-time model is exact only as f_ref/fn grows (Gardner 1980): the
    error must shrink as the reference gets faster for the same loop. The XSPICE PFD
    quantises each pulse to the step h; through R that kicks the reference-averaged
    frequency by Kv*Ip*R*h*f_ref = Kv*Ip*R/ppp. Holding it at a ninth of the bench's
    band/3 (5 % of a 1 % step) needs ppp >= 9*Kv*Ip*R/(5e-4*f_ref) = 4e10/f_ref."""
    errs = []
    for f_ref in (5e6, 10e6, 20e6):
        bench = pll_lock_time(ps, _pll(f_ref), reference_node="ref", output_node="vco", control_node="vc",
                              feedback_node="fb", reference_frequency=f_ref, step_ratio=0.01,
                              pre_cycles=60 * round(f_ref / 5e6), post_cycles=60 * round(f_ref / 5e6),
                              points_per_period=math.ceil(9 * KV * IP * R1 / (5e-4 * f_ref)))
        m = bench.measure()
        df = 0.01 * f_ref
        t = np.linspace(0, 20e-6, 200001)
        fe, te = _linear_pll(df, t, f_ref * 1.01)
        out = np.nonzero(np.abs(fe) > 0.05 * df)[0]
        errs.append((m["lock_time_s"] / t[out[-1] + 1] - 1, m["phase_error_peak_s"] / np.abs(te).max() - 1,
                     m["frequency_overshoot_ratio"] - max(0.0, fe.max() / df)))
    errs = np.abs(np.array(errs))
    # Sampling error of the CT model falls ~(fn/f_ref)**2 (6.6, 1.9, 0.4 % on lock time):
    # demand at least first order on every metric, and < 1 % at f_ref/fn = 80.
    assert np.all(errs[1:] < errs[:-1] / 2), errs
    assert np.all(errs[-1] < 0.01), errs
    assert m["static_phase_error_s"] == pytest.approx(0.0, abs=50e-12)


# ── pll_loop_margins ──

def test_loop_margins_match_the_third_order_closed_form():
    """Type-II loop, R-C1 shunted by C2: angle(L) = -180 + atan(w*T2) - atan(w*T1), so
    PM = atan(wc*T2) - atan(wc*T1). Designed for b = 1 + C1/C2 = 10 at wc = 1/sqrt(T1*T2),
    where tan(PM) = (b-1)/(2*sqrt(b)): 54.9 deg."""
    ip, kv, n, c1, c2, f_ref = 1e-4, 1e8, 8.0, 900e-12, 100e-12, 50e6
    b, k = 1 + c1 / c2, ip * kv / n
    wd = math.sqrt(k * math.sqrt(b) / (c1 + c2))  # |L(wd)| = K*sqrt(b)/(wd**2*(C1+C2)) = 1
    r = math.sqrt(b) / (wd * c1)                   # puts the zero-pole midpoint at wd
    f = ps.Subcircuit("lf", ["cp"])
    f.R(name="z", positive="cp", negative="m", value=r)
    f.C(name="1", positive="m", negative="0", value=c1)
    f.C(name="2", positive="cp", negative="0", value=c2)
    t2, t1 = r * c1, r * c1 * c2 / (c1 + c2)

    def loop(w):
        s = 1j * w
        return k * (1 + s * t2) / (s * s * (c1 + c2) * (1 + s * t1))

    lo, hi = 1.0, 1e10
    for _ in range(200):
        mid = math.sqrt(lo * hi)
        lo, hi = (mid, hi) if abs(loop(mid)) > 1 else (lo, mid)
    wc = math.sqrt(lo * hi)
    pm = math.degrees(math.atan(wc * t2) - math.atan(wc * t1))
    assert pm == pytest.approx(math.degrees(math.atan((b - 1) / (2 * math.sqrt(b)))), abs=1e-6)
    w = np.logspace(2, 10, 800001)
    h = np.abs(loop(w) / (1 + loop(w)))
    bw = w[np.nonzero(h < h[0] / math.sqrt(2))[0][0]] / TAU
    errs = []
    for ppd in (100, 400):
        m = pll_loop_margins(ps, f, ip=ip, kvco_hz_per_v=kv, filter_in="cp", filter_out="cp", divide_ratio=n,
                             reference_frequency=f_ref, points_per_decade=ppd).measure()
        errs.append(abs(m["phase_margin_deg"] - pm))
    # Log-f interpolation of ln|L| and angle between AC points: < 0.05 deg at 100/decade,
    # and it must shrink with density, or it is a definition error.
    assert errs[0] < 0.05 and errs[1] < errs[0], errs
    assert m["crossover_hz"] == pytest.approx(wc / TAU, rel=1e-4, abs=0)
    assert m["closed_loop_bw_hz"] == pytest.approx(bw, rel=1e-4, abs=0)  # reference grid: 1e-5 decade
    assert m["peaking_db"] == pytest.approx(20 * math.log10(h.max() / h[0]), abs=1e-3)
    assert m["crossover_over_ref"] == pytest.approx(wc / TAU / f_ref, rel=1e-4, abs=0)


# ── pll_reference_spur ──

def test_spur_of_a_sinusoidally_modulated_vco():
    """ctl = Vm*sin(2*pi*fm*t): beta = K*Vm/fm, spur = 20*log10(J1/J0) ~ 20*log10(beta/2)."""
    f_ref, f0, k, vm = 10e6, 100e6, 100e6, 1e-3
    d = _vco("fm", f0, k)
    d.SinusoidalVoltageSource(name="ctl", positive="ctl", negative="0", offset=0.0, amplitude=vm, frequency=f_ref)
    bench = pll_reference_spur(ps, d, output_node="out", control_node="ctl", reference_frequency=f_ref,
                               divide_ratio=10, lock_cycles=10, measure_cycles=64)
    m = bench.measure()
    beta = k * vm / f_ref
    assert m["reference_spur_dbc"] == pytest.approx(20 * math.log10(beta / 2), abs=0.01)  # beta**2/8 = 1e-4 dB
    assert m["reference_spur_h2_dbc"] < m["reference_spur_dbc"] - 60  # pure tone: nothing at 2 f_ref


def test_leakage_spur_grows_20_db_per_decade():
    """Each reference cycle the CP replaces I_leak*T of charge: the spur is linear in I_leak
    while beta << 1 (beta**2/8 < 0.01 dB here) and the CP pulse << T."""
    spurs = []
    for leak in (1e-7, 1e-6):
        bench = pll_reference_spur(ps, _pll(10e6, n=8, leak=leak), control_node="vc", divide_ratio=8,
                                   lock_cycles=150, measure_cycles=128)
        spurs.append(bench.measure()["reference_spur_dbc"])
    assert spurs[1] - spurs[0] == pytest.approx(20.0, abs=0.5)


def test_spur_refuses_edges_that_alias_the_reference():
    with pytest.raises(ValueError, match="divide_ratio > 6"):
        pll_reference_spur(ps, _pll(10e6), control_node="vc", divide_ratio=1)


# ── pfd_cp_transfer ──

def _pfd_cp(delta=0.0, t_rst=1e-9, dac_rise=1e-12):
    d = ps.Subcircuit("pfdcp", ["ref", "fb", "cp"])
    _pfd(d, "ref", "fb", t_rst=t_rst, dac_rise=dac_rise)
    d.BI(name="cp", positive="0", negative="cp", expression=f"{IP}*V(upa)-{IP * (1 + delta)}*V(dna)")
    return d


def test_pfd_gain_and_mismatch_offset():
    """UP is wide skew + t_rst, DN wide t_rst: with I_dn = Ip(1+d), current is zero at
    skew = d*t_rst; slopes Ip and Ip(1+d) per second of skew."""
    delta, t_rst = 0.05, 1e-9 + 1e-12  # AND delay + flip-flop reset delay
    # Near zero skew both CP pulses start close together and ngspice puts no time point
    # on the DAC corners: the mean current there errs by a few ps of charge at 200-1000
    # points per period (crossing off by -1.2 / +3.1 ps), 0.05 ps at 4000.
    m = pfd_cp_transfer(ps, _pfd_cp(delta), edge=1e-12, points_per_period=4000).measure()
    # Bound: the 1 ps clock ramps, inside which the PFD sees each edge.
    assert m["static_offset_s"] == pytest.approx(delta * t_rst, abs=1e-12)
    assert m["kpd_a_per_rad"] == pytest.approx(IP * (1 + delta / 2) / TAU, rel=1e-3, abs=0)
    # Intercepts at d*t_rst and d*t_rst/(1+d): their gap is d**2*t_rst/(1+d).
    assert m["dead_zone_s"] == pytest.approx(delta ** 2 * t_rst / (1 + delta), abs=2e-12)
    assert m["linearity_pct"] < 0.05


def test_pfd_dead_zone_from_a_slow_charge_pump_edge():
    """A DAC rising over Tr (falling at once) drives Ip*V: a pulse of width w >= Tr
    delivers Ip*(w - Tr/2), a shorter one Ip*w**2/(2 Tr). With t_rst << Tr the fitted
    lines cross zero at +-(Tr/2 - t_rst + t_rst**2/(2 Tr)): a dead zone ~ Tr - 2 t_rst."""
    # ngspice puts no time point at the DAC ramp's corners: a step h misses <= Ip*h**2/(8 Tr)
    # per corner, 2.6e-4 of the slope over the fitted span at h = 50 ps.
    tr = 1e-9
    skews = np.array([-5, -4, -3, -2, -1.5, -1, -.5, -.2, 0, .2, .5, 1, 1.5, 2, 3, 4, 5]) * 1e-9
    m = pfd_cp_transfer(ps, _pfd_cp(t_rst=1e-12, dac_rise=tr), skews=skews, fit_from=1.5e-9, edge=1e-12,
                       points_per_period=2000).measure()
    # t_rst is the AND delay plus 1-3 ps of flip-flop delays.
    lo, hi = tr - 2 * 4e-12, tr - 2 * 1e-12
    assert lo - 2e-12 <= m["dead_zone_s"] <= hi + 2e-12
    assert m["static_offset_s"] == pytest.approx(0.0, abs=2e-12)  # symmetric
    assert m["kpd_a_per_rad"] == pytest.approx(IP / TAU, rel=1e-3, abs=0)


# ── dll_lock ──

def _dll(kd_ip_over_cf=0.1, ip=5e-5):
    """Two-cell VCDL (kd = 100 ns/V) in a PFD/CP loop on Cf. The DN flip-flop starts set,
    so out_k is compared with ref_k+1: delay locks to T, and each cycle charge
    Ip*(T - d) moves the delay by kd*Ip/Cf*(T - d): e_k+1 = e_k*(1 - kd*Ip/Cf)."""
    cf = 100e-9 * ip / kd_ip_over_cf
    d = _delay_line("dll", charge="2e-5", threshold="V(ctl)")
    _pfd(d, "in", "out", dn_ic=1)
    d.BI(name="cp", positive="0", negative="ctl", expression=f"{ip}*(V(dna)-V(upa))")
    d.C(name="f", positive="ctl", negative="0", value=cf)
    d.R(name="f", positive="ctl", negative="0", value=1e15)
    return d


def test_dll_locks_first_order_with_the_predicted_ratio():
    f_ref, ratio, tol = 10e6, 0.1, 1e-9  # the bench default, 10 max steps
    bench = dll_lock(ps, _dll(ratio), reference_node="in", output_node="out", control_node="ctl",
                     reference_frequency=f_ref, cycles=100, control_initial=0.3)
    res = bench.runner()
    m = bench.metrics(res)
    assert m["locked_harmonic"] == 1.0
    # The loop's own error sequence, from the output edges against the known reference.
    out = np.array(crossings(res.time, res["out"], 0.5, rising=True))
    e = out[out > 0] - np.arange(1, len(out[out > 0]) + 1) / f_ref
    h = 1 / (1000 * f_ref)
    # The PFD sees each output edge at the next time point (<= h late) and the measured
    # edge is interpolated: cycle k's ratio is off by <= 2h/|e_k|.
    big = np.abs(e[:-1]) > 20 * h
    assert big.sum() >= 10
    assert np.all(np.abs(e[1:][big] / e[:-1][big] - (1 - ratio)) < 2 * h / np.abs(e[:-1][big]))
    k_lock = math.ceil(math.log(tol / abs(e[0])) / math.log(1 - ratio))
    assert m["lock_cycles"] == pytest.approx(k_lock, abs=2)  # +-1 edge quantisation, +-1 on the start
    assert m["static_phase_error_s"] == pytest.approx(0.0, abs=2 * h)


# ── pll_lock (design.py) ──

def test_pll_lock_runner_reads_the_locked_frequency():
    """A locked loop's mean output frequency is N*f_ref exactly; the edge-count average
    over the second half (128 periods) errs by <= 2 max steps / span = 7.8e-5."""
    from spicerack.testbenches import pll_lock

    m = pll_lock(ps, _pll(10e6), control_node="vc").measure()
    assert m["frequency_error_ratio"] == pytest.approx(0.0, abs=2 / (200 * 128))
    assert m["control_final_v"] == pytest.approx(0.0, abs=0.01)  # f0 = f_ref: no control needed
