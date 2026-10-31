# Testbenches

`spicerack.testbenches` has ready-made benches: you pass your design, a bench
wraps it in the right stimulus and load, runs the analyses, and returns named
metrics. Each one is checked against a closed-form answer in the test suite.

## Running a bench

```python
import spicerack as ps
from spicerack.testbenches import step_response

rc = ps.Subcircuit("rc", ["vin", "vout"])     # stands in for your amplifier
rc.R("1", "vin", "vout", 1e3)
rc.C("1", "vout", rc.gnd, 100e-12)            # tau = 100 ns

bench = step_response(ps, rc, period=4e-6, v_low=0.0, v_high=1.0)
m = bench.measure()                           # runs the transient, returns a dict
print(m["settle_rise_0p1pct_s"])              # about 6.9 * tau
print(bench.netlist("ngspice"))               # the deck it ran
```

Every bench takes `ps` and your `Subcircuit` first, then keyword arguments
naming your nodes (`input_node=`, `output_node=`, `supply_node=`, ...) and the
conditions (load, bias, sweep range). `bench.measurements` lists the metric
names it returns. A bench either returns all of them or raises; it never
returns a partial dict.

The benches marked "classic" below have no runner: run their testbench
yourself and pass the result to `metrics`:

```python
from spicerack.testbenches import amplifier_voltage_gain

bench = amplifier_voltage_gain(ps, rc, input_node="vin", output_node="vout")
ac = bench.testbench.ac(variation="dec", number_of_points=100, start_frequency=1e3, stop_frequency=1e9)
print(bench.metrics(ac)["bandwidth_hz"])      # 1/(2 pi R C), about 1.59 MHz
```

## Catalog

Names in `<>` expand per spot frequency or step, such as `psrr_db_at_1khz`.

**Amplifiers, loops, noise, ports, RF**

| Bench | Metrics |
| --- | --- |
| `opamp_open_loop` | `a0_db`, `f_p1_hz`, `gbw_hz`, `ugf_hz`, `pm_unity_deg`, `gm_unity_db` |
| `loop_gain` | `t0_db`, `ugf_hz`, `pm_deg`, `gm_db`, `phase_crossover_hz` (Tian injection at a loop break you bring out as two ports) |
| `step_response` | settling at 0.1% and 0.01% (rise and fall), overshoot, slew, `v_final`, `static_error_pct` |
| `noise_spectrum` | output and input-referred density and band rms, 0.1 to 10 Hz peak-to-peak, kT/C-equivalent capacitance |
| `cmrr` | `cmrr_dc_db`, `cmrr_corner_hz`, `a_cm_db`, `cmrr_db_at_<f>` |
| `offset_icmr` | `vos_v`, `icmr_low_v`, `icmr_high_v`, bias and offset currents |
| `port_impedance` | `z_dc_ohm`, `r_parallel_ohm`, `c_parallel_f`, `z_peak_ohm`, `z_peak_hz`, `z_ohm_at_<f>` |
| `output_swing` | `vout_min_v`, `vout_max_v`, `swing_pp_v`, `a_ol_peak_db` |
| `harmonic_distortion` | `thd_db`, `thd_pct`, `hd2_dbc`, `hd3_dbc`, `sfdr_dbc`, `sndr_db` |
| `rf_two_port` | `s11_db`, `s21_db`, `s12_db`, `s22_db`, `nf_db` |
| `rf_compression_ip3` | `gain_db`, `p1db_in_dbm`, `iip3_dbm`, `oip3_dbm`, `im3_slope` |
| `amplifier_voltage_gain` (classic) | `gain_midband_db`, `bandwidth_hz`, `phase_margin_deg` |
| `amplifier_transimpedance` (classic) | `transimpedance_ohm`, `tia_bandwidth_hz` |
| `amplifier_current_gain` (classic) | `current_gain_midband`, `current_gain_bandwidth_hz` |
| `charge_amplifier` (classic) | `q_injected`, `delta_vout`, `droop_rate` |

**Regulators, references, current-mode, charge**

| Bench | Metrics |
| --- | --- |
| `supply_rejection` | `psrr_dc_db`, `psrr_min_db`, `psrr_min_hz`, `psrr_db_at_<f>` |
| `regulator_line_sweep` | `vout_v`, line regulation (V/V and %/V), `dropout_v`, `ground_current_a` |
| `regulator_load_sweep` | `load_regulation_ohm`, `load_regulation_pct`, `quiescent_current_a`, `ground_current_max_load_a` |
| `regulator_step` | `undershoot_v`, `overshoot_v`, `settling_up_s`, `settling_down_s`, `final_shift_v` (load or line step) |
| `supply_startup` | `started`, `startup_time_s`, `startup_overshoot_v`, `inrush_peak_a` (supply ramped from off) |
| `regulator_loop_margins` | worst phase and gain margin over a load and C_out/ESR grid |
| `current_mirror_output` | `iout_a`, `ratio_error`, `rout_ohm`, `compliance_v`, `compliance_rout_v`, `vin_v` |
| `current_reference` | `iref_27c_a`, `tempco_box_ppm_c`, `ptat_intercept_c`, or supply sensitivity |
| `transconductor_gm` | `gm0_s`, `linear_range_v`, `iout_max_a`, `offset_v` |
| `switched_cap_transfer` | `charge_gain`, `leak_factor`, `gain_error`, `settling_time_s` |
| `charge_pump_current` | `i_up_a`, `i_dn_a`, `mismatch_pct_max`, `output_range_v` |
| `bandgap_reference` (classic) | `vref`, `line_regulation_ppm_v` |
| `bandgap_tempco` (classic) | `vref_27c`, `tempco_box_ppm_c`, `tempco_endpoint_ppm_c` |

`average_power(tran, "Vdd", t0, t1, node="vdd")` gives the mean power a supply delivers.

**Data converters**

| Bench | Metrics |
| --- | --- |
| `adc_dynamic` | `sndr_db`, `snr_db`, `thd_db`, `sfdr_dbc`, `enob_bits`, `enob_fs_bits`, `signal_dbfs`, `outlier_count` |
| `adc_static_linearity` | `dnl_max_lsb`, `inl_max_lsb`, `missing_codes`, `offset_error_lsb`, `gain_error` (ramp or sine histogram) |
| `pipeline_stage_residue` | `interstage_gain`, `gain_error`, `residue_max_v`, `threshold_error_max_v` |
| `comparator_threshold` | `offset_v`, `hysteresis_v`, `chatter_count`, `resolution_v` |
| `comparator_delay` | delay at large and small overdrive, `tau_regen_s`, `v_meta_v`, `decision_errors` |
| `comparator_kickback` | `kick_charge_c`, `kick_peak_v`, `kick_residual_v` |
| `dac_transition` | `settling_time_s`, `delay_s`, `slew_rate_v_per_s`, glitch impulse (V s) |
| `dac_dynamic` | `sfdr_dbc`, `thd_db`, `sndr_db`, `enob_bits`, `fundamental_dbfs` |
| `dac_output_impedance` | `zout_dc_ohm`, `zout_at_f_ohm`, `zout_corner_hz` |
| `sample_hold_dynamic` | `sndr_db`, `sfdr_dbc`, `thd_db`, `sampled_gain_db` |
| `sample_hold_track_bandwidth`, `sample_hold_feedthrough` | `track_bandwidth_hz`; `feedthrough_db` |
| `dac_static_linearity`, `adc_ramp`, `sample_hold` (classic) | DNL/INL and monotonicity; missing codes; acquisition, droop, pedestal |

`coherent_bin(M, target_fraction)` picks an input frequency bin for a coherent FFT record.

**Oscillators, PLLs, DLLs, clocks**

| Bench | Metrics |
| --- | --- |
| `oscillator_frequency` | `frequency_hz`, `amplitude_pp_v`, `duty_cycle`, `startup_time_s` |
| `oscillator_sweep` | `f_min_hz`, `f_max_hz`, `tuning_range_ratio`, `df_dv_hz_per_v` (KVCO), `monotonic` |
| `oscillator_jitter` | period and cycle-to-cycle jitter, phase noise at offsets (from a `trnoise` source you add) |
| `crystal_negative_resistance` | `negative_resistance_ohm`, `load_capacitance_f` |
| `pll_lock_time` | `lock_time_s`, `frequency_overshoot_ratio`, `control_final_v`, `control_ripple_v` |
| `pll_loop_margins` | `crossover_hz`, `phase_margin_deg`, `closed_loop_bw_hz`, `peaking_db` |
| `pll_reference_spur` | `reference_spur_dbc`, per-harmonic spurs |
| `pfd_cp_transfer` | `kpd_a_per_rad`, `static_offset_s`, `dead_zone_s`, `linearity_pct` |
| `delay_line_range`, `dll_lock` | delay range and `kd_s_per_v`; static phase error, lock cycles, false-lock harmonic |
| `clock_buffer_timing` | `tplh_s`, `tphl_s`, `dcd_s`, `duty_out`, supply-induced jitter |
| `pll_lock` (classic) | `output_frequency_hz`, `frequency_error_ratio`, `control_final_v` |

Not measured, because SPICE cannot show them honestly: device phase noise and
jitter on ngspice (it adds no device noise to a transient; inject `trnoise`
instead), charge injection from a behavioural switch (it carries no charge), and
start-up from a DC sweep (use `supply_startup`).

## Corners and Monte Carlo

`run_corners` runs one bench per corner and seed and returns one row per run.
`build` makes a fresh bench for a corner; `measure` runs it.

```python
from spicerack.testbenches import CornerCase, run_corners

corners = [CornerCase("cold", temperature=-40), CornerCase("hot", temperature=125)]
rows = run_corners(lambda c: step_response(ps, rc, period=4e-6),
                   lambda bench: bench.measure(),
                   corners)
for row in rows:
    print(row["corner"], row.get("settle_rise_0p1pct_s"), row.get("error"))
```

A `CornerCase` sets `temperature`, `parameters` (`.param` values),
`model_libraries` and `vdd`. `pdk_corners(lib_path, sections, temperatures, vdds)`
builds the full grid for a PDK. `seeds=N` repeats each corner with `.options
seed=1..N`, which varies the statistical (mismatch) models. A run that fails
gets an `"error"` entry instead of metrics, so one bad corner does not lose the
rest. For a yield bound from the pass count, use
[`remeasure.yield_ci`](remeasure.md#yield).

## Checking specs

```python
from spicerack.testbenches import ValidationRule, validate_metrics

report = validate_metrics(m, [
    ValidationRule("settles fast", "settle_rise_0p1pct_s", maximum=1e-6),
    ValidationRule("no overshoot", "overshoot_rise_pct", maximum=1.0),
])
print(report.passed)
```

## Cell characterization

For Liberty tables of standard cells: `input_capacitance(ps, dut, pin=, vdd=)`,
`delay_table(ps, dut, input_pin=, output_pin=, vdd=, slews=[...], loads=[...])`
(NLDM delay and transition), and `setup_time` / `hold_time` (bisection on a
`passes(tran) -> bool` you write).
