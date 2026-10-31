# Built-in testbenches

`from spicerack.testbenches import ...` — recipes that build ordinary
`Testbench` objects. They are recipes, not a second framework: anything a
recipe does, you can do by hand.

Each returns a `DesignBench`:

```python
bench = amplifier_voltage_gain(ps, dut, output_node="vout")
print(bench.netlist("ngspice"))         # the deck
result  = bench.testbench.ac(variation="dec", number_of_points=200,
                             start_frequency=1, stop_frequency=1e6)
metrics = bench.metrics(result)          # {'gain_midband_db': ..., 'bandwidth_hz': ...}
```

Every bench added since the catalog's first section also carries a `runner`, so one call
runs its own analyses (several solves, when the measurement needs them) and extracts:

```python
from spicerack.testbenches import opamp_open_loop
metrics = opamp_open_loop(ps, dut, inp="inp", inn="inn", out="out").measure()
```

## The metric contract

`DesignBench.measurements` lists the metric names the bench claims. `bench.metrics(result)`
either returns all of them or raises:

- `bench.computed` is `False` → `metrics()` raises `NotImplementedError`. The names in
  `measurements` are documented intent that nothing computes yet.
- An extractor that returns fewer keys than declared raises `RuntimeError`.
- `bench.measure()` is `metrics(runner())`; a bench with no `runner` raises
  `NotImplementedError` and you run `bench.testbench` yourself. A bench that needs several
  solves (Tian loop gain, CMRR, S-parameters, a load grid) returns a dict of results from
  its runner, and its extractor takes that dict.

A metric dict missing a key it advertised is worse than an error, because callers index it.
When you wire a new extractor, the contract test in `tests/test_bench_metrics.py` is what
holds it honest.

## Catalog

Every bench computes its declared metrics; `tests/test_bench_metrics.py` fails if
any of them stops doing so.

| Bench | Category | Analysis | Metrics |
|---|---|---|---|
| `amplifier_voltage_gain` | amplifier | op + ac | `gain_midband_db`, `bandwidth_hz`, `phase_margin_deg` |
| `amplifier_transimpedance` | amplifier | op + ac | `transimpedance_ohm`, `tia_bandwidth_hz` |
| `amplifier_current_gain` | amplifier | op + ac | `current_gain_midband`, `current_gain_bandwidth_hz` |
| `charge_amplifier` | amplifier | tran | `q_injected`, `delta_vout`, `droop_rate` |
| `sample_hold` | sample_hold | tran | `acquisition_error`, `hold_droop`, `pedestal_step` |
| `dac_static_linearity` | dac | tran | `offset_error_lsb`, `gain_error`, `dnl_max_lsb`, `inl_max_lsb`, `monotonic` |
| `adc_ramp` | adc | tran | `codes_seen`, `missing_codes`, `transition_count`, `code_monotonic` |
| `switch_characterization` | switch | dc | `ron_ohm`, `roff_ohm`, `off_isolation_db` |
| `mux_routing` | mux | tran | `selected_gain_min/max`, `channel_gain_mismatch`, `all_channels_routed` |
| `demux_routing` | demux | tran | `selected_gain_min`, `inactive_output_max`, `off_isolation_db` |
| `pll_lock` | pll | tran | `output_frequency_hz`, `frequency_error_ratio`, `control_settling_v`, `control_final_v` |
| `bandgap_reference` | bandgap | dc | `vref`, `line_regulation_ppm_v` |
| `bandgap_tempco` | bandgap | dc(temp) | `vref_27c`, `tempco_box_ppm_c`, `tempco_endpoint_ppm_c` |

### Amplifiers, loops, noise, ports, RF

| Bench | Category | Analysis | Metrics |
|---|---|---|---|
| `opamp_open_loop` | opamp | op + ac (L-C DC servo) | `a0_db`, `f_p1_hz`, `gbw_hz`, `ugf_hz`, `pm_unity_deg`, `gm_unity_db` |
| `loop_gain` | stability | ac ×2 (Tian) | `t0_db`, `ugf_hz`, `pm_deg`, `gm_db`, `phase_crossover_hz` |
| `step_response` | opamp | tran | `settle_{rise,fall}_{0p1,0p01}pct_s`, `overshoot_{rise,fall}_pct`, `v_final`, `static_error_pct`, `slew_{rise,fall}_v_per_s` |
| `noise_spectrum` | noise | op + noise | `out_rms_band_v`, `in_rms_band`, `out_vpp_0p1_10hz` (6.6 × rms), `out_rms_total_v`, `noise_equivalent_c_f`, `out_density_at_<f>`, `in_density_at_<f>` |
| `cmrr` | opamp | ac ×2 (dm, cm) | `cmrr_dc_db`, `cmrr_corner_hz`, `a_cm_db`, `cmrr_db_at_<f>` |
| `offset_icmr` | opamp | dc | `vos_v`, `icmr_low_v`, `icmr_high_v`, `ib_pos_a`, `ib_neg_a`, `ios_a` |
| `port_impedance` | port | op + ac (voltage or current drive) | `z_dc_ohm`, `r_parallel_ohm`, `c_parallel_f`, `z_peak_ohm`, `z_peak_hz`, `z_ohm_at_<f>`, `z_phase_deg_at_<f>` |
| `output_swing` | opamp | dc | `vout_min_v`, `vout_max_v`, `swing_pp_v`, `a_ol_peak_db` |
| `harmonic_distortion` | linearity | tran (coherent) | `thd_db`, `thd_pct`, `hd2_dbc`, `hd3_dbc`, `sfdr_dbc`, `sndr_db` |
| `rf_two_port` | rf | ac ×2 + noise | `s11_db`, `s21_db`, `s12_db`, `s22_db`, `nf_db` (at 290 K) |
| `rf_compression_ip3` | rf | tran × levels | `gain_db`, `p1db_in_dbm`, `iip3_dbm`, `oip3_dbm`, `im3_slope` |

`loop_gain` breaks the loop at two DUT ports, `e` (driven side) and `f` (driving side), and
injects twice (Tian 2001): exact whatever the impedance at the break, on any backend. PSRR
is `supply_rejection` below.

### Regulators, references, current-mode, charge

| Bench | Category | Analysis | Metrics |
|---|---|---|---|
| `supply_rejection` | power | op + ac | `psrr_dc_db`, `psrr_min_db`, `psrr_min_hz`, `psrr_db_at_<f>` |
| `regulator_line_sweep` | power | dc (V_in) | `vout_v`, `line_regulation_v_per_v`, `line_regulation_pct_per_v`, `dropout_v`, `ground_current_a` |
| `regulator_load_sweep` | power | dc (I_load) | `load_regulation_ohm`, `load_regulation_pct`, `quiescent_current_a`, `ground_current_max_load_a` |
| `regulator_step` | power | tran (load or line step) | `undershoot_v`, `overshoot_v`, `settling_up_s`, `settling_down_s`, `final_shift_v` |
| `supply_startup` | power | tran (supply ramp, no `uic`) | `started`, `startup_time_s`, `startup_after_supply_s`, `startup_overshoot_v`, `inrush_peak_a` |
| `regulator_loop_margins` | power | ac ×2 per grid point (Tian) | `phase_margin_deg_min`, `gain_margin_db_min`, `unity_gain_hz_at_worst`, `worst_load_a`, `worst_cout_f`, `worst_esr_ohm` |
| `current_mirror_output` | current_mode | dc (V_out) | `iout_a`, `ratio_error`, `rout_ohm`, `compliance_v`, `compliance_rout_v`, `vin_v` |
| `current_reference` | current_mode | dc temp / dc V_dd | `iref_27c_a`, `tempco_box_ppm_c`, `ptat_intercept_c` / `iref_a`, `supply_sensitivity_pct_per_v` |
| `transconductor_gm` | current_mode | dc (V_id) | `gm0_s`, `linear_range_v`, `iout_max_a`, `offset_v` |
| `switched_cap_transfer` | charge | tran (non-overlapping clocks) | `charge_gain`, `leak_factor`, `gain_error`, `settling_time_s` |
| `charge_pump_current` | charge | dc ×3 (UP, DN, both) | `i_up_a`, `i_dn_a`, `mismatch_pct_max`, `output_range_v`, `both_on_net_a_at_mid` |

`average_power(tran, supply_source, t0, t1, node=...)` is a helper: the trapezoid of
`v·(−i)` over an integer number of periods. `amplifier_current_gain` takes
`input_bias_current` and `output_bias_voltage`; without them a mirror is unbiased and reads
a gain of about 0.

### Data converters

| Bench | Category | Analysis | Metrics |
|---|---|---|---|
| `adc_dynamic` | adc | tran (coherent sine) | `sndr_db`, `snr_db`, `thd_db`, `sfdr_dbc`, `hd2_db`, `hd3_db`, `enob_bits`, `enob_fs_bits`, `signal_dbfs`, `input_frequency_hz`, `outlier_count` |
| `adc_static_linearity` | adc | tran (ramp or sine histogram) | `dnl_max_lsb`, `dnl_min_lsb`, `inl_max_lsb`, `missing_codes`, `offset_error_lsb`, `gain_error` |
| `pipeline_stage_residue` | adc | tran | `interstage_gain`, `gain_error`, `residue_max_v`, `overrange_margin_v`, `threshold_error_max_v`, `residue_jump_v` |
| `comparator_threshold` | comparator | tran (up-down ramp) | `threshold_up_v`, `threshold_down_v`, `offset_v`, `hysteresis_v`, `chatter_count`, `resolution_v` |
| `comparator_delay` | comparator | tran (alternating overdrive) | `delay_max_od_s`, `delay_min_od_s`, `tau_regen_s`, `v_meta_v`, `decision_errors`, `delay_sign_skew_s` |
| `comparator_kickback` | comparator | tran | `kick_charge_c`, `kick_peak_v`, `kick_diff_peak_v`, `kick_residual_v` |
| `dac_transition` | dac | tran | `settling_time_s`, `settling_time_carry_s`, `delay_s`, `slew_rate_v_per_s`, `glitch_impulse_vs`, `glitch_impulse_abs_vs`, `glitch_peak_v` |
| `dac_dynamic` | dac | tran (sine code sequence) | `sfdr_dbc`, `thd_db`, `sndr_db`, `snr_db`, `hd2_db`, `hd3_db`, `enob_bits`, `fundamental_dbfs` |
| `dac_output_impedance` | dac | ac | `zout_dc_ohm`, `zout_at_f_ohm`, `zout_corner_hz` |
| `sample_hold_dynamic` | sample_hold | tran | `sndr_db`, `snr_db`, `sfdr_dbc`, `thd_db`, `hd2_db`, `hd3_db`, `enob_bits`, `sampled_gain_db` |
| `sample_hold_track_bandwidth` | sample_hold | ac (clock held in track) | `track_bandwidth_hz`, `track_gain_db` |
| `sample_hold_feedthrough` | sample_hold | ac (clock held in hold) | `feedthrough_db`, `feedthrough_max_db` |

`coherent_bin(M, target_fraction)` picks a prime input bin so every sample lands on a
distinct phase and no harmonic folds onto the fundamental. Spectra read **held** nodes:
interpolating a moving node off the solver grid caps SNDR at about 72 dB at 100 solver
points per input period.

### Oscillators, PLLs, DLLs, clocks

| Bench | Category | Analysis | Metrics |
|---|---|---|---|
| `oscillator_frequency` | oscillator | tran | `frequency_hz`, `period_spread_s`, `amplitude_pp_v`, `duty_cycle`, `startup_time_s`, (`supply_current_a`) |
| `oscillator_sweep` | vco | tran (staircase) | `frequency_hz_<k>`, `f_min_hz`, `f_max_hz`, `tuning_range_ratio`, `df_dv_hz_per_v`, `df_dv_max_over_min`, `monotonic` |
| `oscillator_jitter` | oscillator | tran + injected `trnoise` | `frequency_hz`, `period_jitter_rms_s`, `period_jitter_pp_s`, `cycle_to_cycle_jitter_rms_s`, `c_s`, `jitter_growth_exponent`, `phase_noise_dbc_hz_at_<offset>` |
| `crystal_negative_resistance` | oscillator | op + ac | `negative_resistance_ohm`, `load_capacitance_f`, (`negative_resistance_margin`) |
| `pll_lock_time` | pll | tran | `lock_time_s`, `frequency_overshoot_ratio`, `control_final_v`, `control_ripple_v`, (`static_phase_error_s`, `phase_error_peak_s`) |
| `pll_loop_margins` | pll | op + ac (phase-domain model) | `crossover_hz`, `phase_margin_deg`, `closed_loop_bw_hz`, `peaking_db`, `crossover_over_ref` |
| `pll_reference_spur` | pll | tran | `reference_spur_dbc`, `reference_spur_h<k>_dbc`, `control_ripple_v` |
| `pfd_cp_transfer` | pll | tran (skew staircase) | `kpd_a_per_rad`, `static_offset_s`, `dead_zone_s`, `linearity_pct` |
| `delay_line_range` | dll | tran (staircase) | `delay_s_<k>`, `delay_min_s`, `delay_max_s`, `kd_s_per_v`, `covers_period`, `harmonic_lock_risk` |
| `dll_lock` | dll | tran | `static_phase_error_s`, `locked_harmonic`, `lock_cycles`, `control_final_v` |
| `clock_buffer_timing` | clock | tran | `tplh_s`, `tphl_s`, `dcd_s`, `duty_out`, (`psij_pp_s`, `psij_s_per_v`) |

Metrics in parentheses appear only when the bench is given the extra node or stimulus they
need. `oscillator_jitter` raises when the deck carries no noise source rather than report
0 s, and `oscillator_frequency` raises on a latched output rather than report 0 Hz.

`phase_margin_deg` is `nan` when the response never crosses 0 dB, and is a stability
margin only on an open-loop response.

`ac_bandwidth_hz` raises when the response never falls below its passband within the
swept range. That is a bench-configuration signal — widen the sweep — not a value to
paper over with a default.

### What these benches deliberately do not measure

Each is unreachable with the analyses available, and a bench that reported a number
anyway would be reporting its own setup:

- **Charge injection** (`switch_characterization`) — a behavioural ngspice `S`/`W`
  element carries no channel charge and would report exactly zero, passing every time.
  Needs a transistor-level switch.
- **Aperture delay** (`sample_hold`) — inseparable from the pedestal in a single
  record; needs two runs with opposite input slopes.
- **Device phase noise and jitter** — ngspice injects no device noise into a transient,
  and PSS + pnoise runs on Spectre only. `oscillator_jitter` measures jitter from a
  `trnoise` source you inject, never from device noise. Reference spurs are deterministic,
  so `pll_reference_spur` reads them from the VCO-edge timing error of a noiseless locked
  transient, without `.four`. That needs N > 2 × the highest harmonic, because the edges
  sample the phase at N·f_ref.
- **Delta-sigma in-band SQNR, comparator noise** — the first needs a band-limited,
  Hann-windowed spectrum core; the second needs device transient noise (VACASK only).
- **Startup margin** (`bandgap_reference`) — a DC sweep is a continuation and can
  march along the degenerate zero-current branch; only a transient from a genuinely
  off state proves startup.
- **Latency** (`adc_ramp`) — needs a DUT with a known pipeline depth to align against.

## Validation and corners

```python
from spicerack.testbenches import (
    ValidationRule, validate_metrics, MetricSpec, extract_metrics,
    CornerCase, corner_netlists, MonteCarloPlan, monte_carlo_netlist,
)

report = validate_metrics(metrics, [
    ValidationRule("gain target", "gain_midband_db", expected=40.0, tolerance=0.5),
    ValidationRule("bandwidth floor", "bandwidth_hz", minimum=1e6),
])
assert report.passed
```

`DesignBench.validation` is a `list[str]` of English intent, **not** `ValidationRule`
objects — build the rules yourself as above.

`extract_metrics(result, specs)` reduces waveforms with `MetricSpec(name, source, reducer)`,
reducers: `first last min max mean abs_max peak_to_peak at crossing_time`.

`corner_netlists(bench_factory, [CornerCase(...)])` returns `{corner_name: netlist}`.
`monte_carlo_netlist` is Spectre-only — real signatures and the ngspice route are in [`PDK_AND_FLOW.md`](PDK_AND_FLOW.md).

## Corner / Monte Carlo runner

```python
from spicerack.testbenches import pdk_corners, run_corners

def build(corner):                      # fresh bench per run; reads corner.vdd itself
    tb = ps.Testbench(dut)
    tb.V(name="dd", positive="vdd", negative="0", value=corner.vdd or 1.8)
    return tb

def measure(tb):                        # runs the analysis, returns {metric: value}
    return {"vout_v": tb.operating_point()["vout"]}

corners = pdk_corners(LIB, ["tt", "ss", "ff"], temperatures=[-40, 27, 125], vdds=[1.62, 1.98])
rows = run_corners(build, measure, corners)                  # one row per corner
rows = run_corners(build, measure, pdk_corners(LIB, ["tt_mm"]), seeds=100)   # mismatch MC
```

* `run_corners(build, measure, corners=(), *, seeds=None, backend="ngspice") -> list[dict]`.
  Each row is `{"corner", "seed", "temperature", "vdd", **metrics}`; a run that raises gets
  `"error": "<Type>: <message>"` instead of metrics, so filter on it.
* After `build(corner)` it applies the corner's temperature, model libraries and
  `.param` values, and `.options seed=<n>` per seed. `seeds=N` means `1..N`.
* A seed changes nothing unless the models are statistical (a PDK mismatch section or
  `agauss`). ngspice draws are reproducible per seed.
* `pdk_corners(library, sections, temperatures=(27,), vdds=(None,))` crosses them into
  `CornerCase`s named `ss_125C_1.62V`. `CornerCase.vdd` is read by your builder; nothing
  else applies it, because only the builder knows which source is the supply.
* Runs are sequential. A sky130 run spends ~7 s loading the model library, so
  45 corners x 100 seeds is hours.

## Cell characterization (for Liberty)

`from spicerack.testbenches import input_capacitance, delay_table, setup_time, hold_time, bisect_boundary`

The DUT is a `Subcircuit` whose ports are the cell pins. `bias={pin: volts}` drives the
pins a recipe does not (supplies, other inputs); `loads={pin: farads}` adds caps to
ground. Everything returns SI values with the unit in the key.

| Call | Returns |
|---|---|
| `input_capacitance(ps, dut, *, pin, vdd, slew=100e-12, settle=None, bias=, loads=)` | `{"c_rise_f", "c_fall_f"}` |
| `delay_table(ps, dut, *, input_pin, output_pin, vdd, slews, loads, window=10e-9, slew_lower=0.2, slew_upper=0.8, delay_threshold=0.5, bias=)` | `{"index_1_s", "index_2_f", "cell_rise_s", "cell_fall_s", "rise_transition_s", "fall_transition_s"}` |
| `setup_time(ps, dut, *, data_pin, clock_pin, vdd, passes, search=(-1e-9, 1e-9), tol=1e-12, data_edge="rise", clock_edge="rise", slew=50e-12, clock_time=5e-9, window=5e-9, bias=, loads=)` | `{"setup_s"}` |
| `hold_time(...)` (same arguments, `data_edge="fall"`) | `{"hold_s"}` |
| `bisect_boundary(passes, lo, hi, tol)` | smallest passing `x` |

* **Pin cap** is the charge method: `C = -∫i dt / ΔV` over each ramp plus `settle`
  (default one `slew`). Charge behind a series resistance arrives late; widen `settle`
  if the answer moves with it. DC leakage counts as charge.
* **NLDM tables** are `table[i][j]` at `slews[i]`, `loads[j]`, keyed by **output** edge
  (Liberty's convention), so inverting cells need no flag. `slew` is the
  `slew_lower`–`slew_upper` time; the ramp itself lasts `slew / (upper - lower)`.
  `window` must cover the slowest output edge, or it raises `widen window=`.
* **Setup/hold** bisect the data-to-clock skew on `passes(tran) -> bool`, which you
  write (e.g. "q equals the new data at the end"). Setup: data makes `data_edge` at
  `clock_time - skew`. Hold: data leaves its old value at `clock_time + skew` (negative
  hold is fine). Edges are referenced at 50 %. `passes` must be true at `search[1]` and
  false at `search[0]`, or it raises rather than return a bracket end.

## Writing a new bench

**The load is the specification.** One netlist cannot measure several specs, because each
spec mandates a different termination — charge injection wants a 1 nF hold cap and no
resistive load; off-isolation wants 50 Ω ∥ 5 pF; timing wants 100 Ω ∥ 35 pF; on-resistance
wants a forced current and no load at all. Benches that try to serve several specs at once
end up declaring metrics that have no observable in the deck. Prefer several small
single-purpose benches; each is smaller than the composite would have to be, and each has
one unambiguous extraction.

Two concrete failures of this kind in the current catalog, worth reading before you add one:
`switch_characterization` drives its input from an ideal 0 Ω source into a capacitor-only
load, so no DC current flows and on-resistance is unobservable for any value; `mux_routing`
drives every input from an ideal source, so a momentary two-channel overlap produces no
measurable current anywhere.

The shape:

```python
def my_bench(ps, dut, *, output_node="vout") -> DesignBench:
    tb = ps.Testbench(dut)
    tb.V(name="in", positive="vin", negative="0", value=0.0, ac=1.0)
    tb.save(f"V({output_node})")
    tb.add_ac(variation="dec", number_of_points=100,
              start_frequency=1.0, stop_frequency=1e9)

    def extract(ac):
        from .analysis import ac_bandwidth_hz, ac_passband_db
        return {
            "gain_db": ac_passband_db(ac, output_node),
            "bw_hz": ac_bandwidth_hz(ac, output_node),
        }

    return DesignBench(
        name="my_bench", category="amplifier", testbench=tb,
        intent="What this bench is for.",
        measurements=["gain_db", "bw_hz"],       # must match extract()'s keys
        result_fields=["ac.frequency"],
        validation=["gain in budget"],
        extractor=extract,
    )
```

Pitfalls the current catalog hit, so the next bench does not:

- `pytest.approx(x, rel=r)` keeps `abs=1e-12`, which governs on pF, fC, nA, nV/√Hz and ps
  values and passes a 2× error. Write `rel=r, abs=0`.
- ngspice's `S` switch is on at the operating point but stamps `roff` in `.ac`: model an AC
  switch as a B-source conductance.
- ngspice's `^` is not a signed power on a negative base: write `V(x)*V(x)*V(x)`.
- A huge C between two driven nodes (an "AC short") misreads on ngspice; use an L-C
  low-pass to ground and a series AC source instead.
- `.tran` without a max step lets the simulator choose the grid; every bench sets one.
- At the default `reltol=1e-3`, behavioural B-source oracles are off by up to (ωh)²/2; oracle
  DUTs set `.options reltol=1e-6`.
- ngspice-45 `trnoise` ignores every seed option, so two runs differ: jitter tests are
  statistical (≥ 3σ of the estimator), never exact.

Validate a new extractor against a circuit with a closed-form answer — an RC low-pass
gives an exact corner at `1/(2*pi*R*C)` — and check that the error *shrinks* as you raise
sweep density. An error that stays put under refinement is a definition bug, not a
numerical one.
