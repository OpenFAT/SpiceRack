# Python API gaps: waveform sources and analysis builders vs PySpice

Sources:
- PySpice at commit `fe08718d5a1a9af94576b5aef1fa81910f09b037` (2024-01-26), cloned from
  https://github.com/PySpice-org/PySpice. Paths below are relative to `PySpice/Spice/`.
- SpiceRack at `ed184dc` (branch `dialect-import`). Paths relative to the repo root.

## 1. What PySpice does

### How element methods attach

Every class in `BasicElement` and `HighLevelElement` is attached to `Netlist` as a method named
after the class (`__init__.py:52-65`). `Circuit` and `SubCircuit` both derive from `Netlist`, so
**every source exists on both, with identical signatures**. Each method is called as
`netlist.Cls(name, node_plus, node_minus, *args, **kwargs)`. The name and the two nodes are
**positional** (`HighLevelElement.py:587`).

### Waveform sources (HighLevelElement.py)

| PySpice class (V and I variants) | Mixin `__init__` signature | Emitted card |
|---|---|---|
| `SinusoidalVoltageSource` / `SinusoidalCurrentSource` | `dc_offset=0, ac_magnitude=1, offset=0, amplitude=1, frequency=50, delay=0, damping_factor=0` (`:102-106`) | `DC {dc_offset} AC {ac_magnitude} SIN(offset amplitude frequency delay damping_factor)` (`:131-137`) |
| `PulseVoltageSource` / `PulseCurrentSource` | `initial_value, pulsed_value, pulse_width, period, delay_time=0, rise_time=0, fall_time=0, phase=None, dc_offset=0` (`:215-220`). `phase` is XSPICE-only (`:235-239`) | `DC {dc_offset} PULSE(initial pulsed delay_time rise fall width period [phase])` (`:260-273`) |
| `PieceWiseLinearVoltageSource` / `PieceWiseLinearCurrentSource` | `values, repeat_time=None, delay_time=None, dc=None` (`:377`) | `[DC {dc}] PWL(t v ... [r=repeat_time] [td=delay_time])` (`:385-404`) |
| `ExponentialVoltageSource` / `ExponentialCurrentSource` | `initial_value, pulsed_value, rise_delay_time=.0, rise_time_constant=None, fall_delay_time=None, fall_time_constant=None` (`:320-323`) | `EXP(...)`, with no DC and no AC (`:336-344`) |
| `SingleFrequencyFMVoltageSource` / `SingleFrequencyFMCurrentSource` | `offset, amplitude, carrier_frequency, modulation_index, signal_frequency`, all required (`:439`) | `SFFM(offset amplitude carrier_frequency modulation_index signal_frequency)` (`:448-453`) |
| `AmplitudeModulatedVoltageSource` / `AmplitudeModulatedCurrentSource` | `offset, amplitude, modulating_frequency, carrier_frequency, signal_delay`, all required (`:489`) | `AM(offset amplitude carrier_frequency modulating_frequency signal_delay)` (`:501-506`) |

Notes:
- **DC alongside a waveform.** Sin and Pulse use `dc_offset`, and PWL uses `dc`. Exp, SFFM and AM
  have no DC parameter.
- **AC alongside a waveform.** Only Sin has it, as `ac_magnitude`, and it defaults to **1**, so
  PySpice always writes `AC 1` on sine sources (`:104`, `:134`). No source has an AC phase
  parameter. Pulse, PWL, Exp, SFFM and AM have no AC parameter.
- **Plain `V`/`I`.** These take one positional `dc_value` (`BasicElement.py:787`, `:813`) and have
  no `ac=` keyword.
- **PySpice AM bug.** The AM docstring gives the SPICE syntax as `AM(VA VO MF FC TD)`
  (`HighLevelElement.py:477`), but `format_spice_parameters` writes offset first and the carrier
  before the modulating frequency (`:501-506`). That swaps two pairs of positions. SpiceRack should
  match by **keyword name** and must not copy PySpice's positional order. Spectre already refuses
  AM (`src/codegen/spectre.rs:202-206`).
- `AcLine(rms_voltage=230, frequency=50)` is a Sin subclass (`HighLevelElement.py:617-621`). It is
  not needed here.

### Analyses (Simulation.py)

`CircuitSimulation` registers analyses with `_add_analysis`, which keys them by name, so a second
call of the same analysis replaces the first (`:702-704`). `CircuitSimulator` wraps each analysis
in `_run` and runs it immediately (`:1173-1236`).

| Analysis | PySpice method | Line |
|---|---|---|
| `.tf` | `transfer_function(outvar, insrc)`, alias `tf = transfer_function` | `:989`, `:1233-1236` |
| `.sens` (DC) | `dc_sensitivity(output_variable)` | `:718`, run wrapper `:1203` |
| `.sens ac` | `ac_sensitivity(output_variable, variation, number_of_points, start_frequency, stop_frequency)` | `:746`. It has **no** `_run` wrapper in `CircuitSimulator` (`:1193-1236`) |
| `.pz` | `polezero(node1, node2, node3, node4, tf_type, pz_type)` | `:902`, `:1218` |
| `.disto` | `distortion(variation, points, start_frequency, stop_frequency, f2overf1=None)` | `:1023`, `:1228` |

PySpice has no `add_*` names. Its build-only form is the base-class method (the
`CircuitSimulation.*` methods above), which only appends the analysis.

## 2. SpiceRack inventory (src/python.rs)

`Circuit` extends `Subcircuit` (`:241`), so `Circuit` has exactly the source methods `Subcircuit`
has. `Testbench` extends `CircuitSimulator` (`:1315`) and has its own copies of the stimulus
methods (`:1341-1408`). All of them are keyword-only (`*, name, positive, negative, ...`).

| Method | Subcircuit / Circuit | Testbench |
|---|---|---|
| `V`, `I` (`value, ac=None, ac_phase=None`) | `:1014-1022` | `:1341-1349` |
| `SinusoidalVoltageSource(dc_offset=0, offset=0, amplitude=1, frequency=1000, ac=None, ac_phase=None)` | `:1152` | `:1351` |
| `SinusoidalCurrentSource` | `:1175`, **no `ac`/`ac_phase`** | `:1375`, has `ac`/`ac_phase` |
| `PulseVoltageSource` / `PulseCurrentSource(initial_value=0, pulsed_value=1, delay=0, pulse_width=50e-9, period=100e-9, rise_time=1e-9, fall_time=1e-9)` | `:1160`, `:1183`. No dc, no ac | `:1360`, `:1384`. No dc, no ac |
| `PieceWiseLinearVoltageSource(values)` | `:1169`. No dc, no ac | `:1369`. No dc, no ac |
| `PieceWiseLinearCurrentSource(values)` | **missing** | `:1393` |
| Exp / SFFM / AM sources | **missing** | **missing** |
| `R` | `value, raw_spice=None, **kwargs` (`:987`) | `value` only (`:1399`) |
| `C` | `value, **kwargs` (`:997`) | `value` only (`:1404`) |
| `L`, `E`, `G`, `F`, `H`, `BV`, `BI`, ... | yes | **missing** (unverified that anyone needs them in a testbench) |
| Node names | `src()` goes through `net()`, which maps `gnd` to `0` only on `Circuit` (`:945-957`) | `positive.into()` raw, with no `gnd` alias (`:1343`) |

Other facts:
- All waveform parameters are `f64` (`:1153-1190`). Plain `V`/`I` accept `PyValueArg`, which can be
  a float, a UnitValue or an expression (`:147-167`). The waveform fields in the IR are `f64`
  (`src/ir/mod.rs:157-164`), so **parameter expressions inside waveforms need an IR change**.
- Pulse and PWL compute the DC value as `at_zero(wf)`, the waveform's value at t=0 (`:182-192`).
  Sin uses `num(dc_offset)`.
- `sin()` hard-codes `delay: 0.0, damping: 0.0, phase: 0.0` (`:174-176`), although the IR has all
  three fields (`src/ir/mod.rs:158`).

Analyses on `CircuitSimulator`, which run immediately: `transfer_function`/`tf` (`:505-514`),
`dc_sensitivity` (`:516`), `ac_sensitivity` (`:522`), `polezero` (`:532`), `distortion` (`:543`).
`pss`, `s_param`, `network_params`, `harmonic_balance`, `stability` and `transient_noise` also run
immediately (`:551-600`). The deck builders on `Testbench` are `add_operating_point`, `add_dc`,
`add_ac`, `add_transient`, `add_noise`, `add_fourier`, `add_spectre_sweep` and
`add_spectre_monte_carlo`/`montecarlo` (`:1423-1483`). There is **no** `add_tf`, `add_*sensitivity`,
`add_polezero` or `add_distortion`, and also no `add_pss`/`add_s_param`/`add_harmonic_balance`/
`add_stability`/`add_transient_noise`.

**The IR and codegen already carry DC + AC + waveform together.** The source component has
`value`, `ac_magnitude`, `ac_phase` and `waveform` (`src/ir/mod.rs:105-106`). spice3 codegen writes
`V.. np nm <value> [AC mag [phase]] <WAVEFORM>` (`src/codegen/spice3.rs:341-355`), and VACASK
writes `mag=`/`phase=` (`src/codegen/vacask.rs:238-246`, `:817-837`). The DC and AC gaps are
therefore purely in the Python layer.

## 3. Proposed changes

These are backward compatible: every current kwarg keeps its name and default. Where PySpice has a
name, the proposal uses it. Each item is listed with the IR field it fills.

**A. One signature per source on all three classes.** Implement each source once in a shared
helper that returns a `Component`. `Subcircuit.push(net(...))` and `Testbench.stim(...)` both call
it. This removes the duplicated bodies at `:1152-1190` and `:1351-1397`, and fixes the missing
`ac` on `SinusoidalCurrentSource` and the missing Subcircuit `PieceWiseLinearCurrentSource`.

**B. Positional `name, positive, negative`.** Change the signature from `(*, name, positive,
negative, ...)` to `(name, positive, negative, *, ...)`. PySpice calls pass these three
positionally (`HighLevelElement.py:587`). Without this change, code like
`circuit.PulseVoltageSource('in', 'in', circuit.gnd, ...)` does not port, which contradicts the
claim in README.md:10. Existing keyword calls keep working.

**C. Every waveform source takes `dc=None, ac=None, ac_phase=None`.**
- `dc` sets `value`. When it is `None`, keep today's `at_zero(wf)` default, extended to Exp
  (returns `initial`) and SFFM (returns `offset`).
- `ac` sets `ac_magnitude` and `ac_phase` sets `ac_phase`.
- PySpice spellings are accepted as aliases. `dc_offset` (Sin, already present; add it to Pulse,
  following PySpice). `dc` (PWL). `ac_magnitude` (Sin), which sets the same field as `ac`. Passing
  both spellings raises `TypeError`.
- Keep the default `ac=None`. PySpice's implicit `AC 1` (`:104`) would change existing SpiceRack
  decks.

**D. Sinusoidal:** add `delay=0.0`, `damping_factor=0.0` and `phase=0.0`, mapping to
`Sin{delay, damping, phase}`. `phase` has no PySpice counterpart, but the IR has the field.

**E. Pulse:** add `delay_time` as a PySpice alias of `delay`, mapping to `Pulse.delay`. PySpice's
XSPICE `phase` has no IR field. Skip it.

**F. PWL:** add the `dc`/`ac`/`ac_phase` keywords from C. PySpice's `repeat_time` and `delay_time`
(`r=`, `td=`) **need an IR change**: `Pwl{values}` has no such fields (`src/ir/mod.rs:160`). They
can be added later as `Option<f64>` with `#[serde(default)]`.

**G. New sources, each with a V and an I variant on all classes:**
- `ExponentialVoltageSource` / `ExponentialCurrentSource`:
  `(initial_value, pulsed_value, rise_delay_time=0.0, rise_time_constant, fall_delay_time, fall_time_constant, dc=None, ac=None, ac_phase=None)`,
  mapping to `Exp{initial, pulsed, rise_delay, rise_tau, fall_delay, fall_tau}`. PySpice defaults
  the last three to `None`, meaning SPICE's TSTEP-relative defaults. The IR fields are plain `f64`,
  so these three are **required** until the IR fields become `Option<f64>`, which would be an IR
  change.
- `SingleFrequencyFMVoltageSource` / `SingleFrequencyFMCurrentSource`:
  `(offset, amplitude, carrier_frequency, modulation_index, signal_frequency, dc=None, ac=None, ac_phase=None)`,
  mapping to `Sffm{offset, amplitude, carrier_freq, modulation_index, signal_freq}`.
- `AmplitudeModulatedVoltageSource` / `AmplitudeModulatedCurrentSource`:
  `(offset, amplitude, modulating_frequency, carrier_frequency, signal_delay, dc=None, ac=None, ac_phase=None)`,
  mapping by name to `Am{offset, amplitude, modulating_freq, carrier_freq, delay}`. Do not copy
  PySpice's emit order.

**H. Testbench analysis builders.** Defaults copy the immediate-run methods at `:505-549`.
- `add_transfer_function(outvar, insrc)`, plus an alias `add_tf`, maps to
  `Analysis::Tf{output, source}`.
- `add_dc_sensitivity(output_variable)` maps to `Analysis::Sensitivity{output, ac: None}`.
- `add_ac_sensitivity(output_variable, variation="dec", number_of_points=10, start_frequency=100.0, stop_frequency=1e5)`
  maps to `Analysis::Sensitivity{output, ac: Some(AcSweepParams{variation, points, start, stop})}`.
- `add_polezero(node1, node2, node3, node4, tf_type, pz_type)` maps to
  `Analysis::PoleZero{node1..node4, tf_type, pz_type}`.
- `add_distortion(variation="dec", points=10, start_frequency=100.0, stop_frequency=1e8, f2overf1=None)`
  maps to `Analysis::Distortion{variation, points, start, stop, f2overf1}`.

  The IR variants are at `src/ir/mod.rs:205-208` and `AcSweepParams` is at `:262-267`. None of
  these builders needs an IR change. Each one is a single `tb().analyses.push(...)`. Codegen
  support already exists: spice3 `.tf` is unconditional (`src/codegen/spice3.rs:484`), while
  `.sens`, `.pz` and `.disto` are limited to ngspice or HSPICE (`:502`, `:509`, `:512`). Extending
  this to pss, s_param, hb, stability and tran-noise is the same one-line pattern, but it is out of
  scope here.

**I. Testbench `R`/`C`:** accept `**kwargs` instance parameters like `Subcircuit` does (`:987-1001`).
Optionally map the string `gnd` to `0` in stimulus nets. This is minor (unverified whether any
caller passes the literal `"gnd"`).

**Needs an IR change:** PWL `repeat_time`/`delay_time`; optional Exp time constants; expression
values in waveform fields (all `f64` today); Pulse XSPICE `phase`. Everything else maps onto
existing fields.
