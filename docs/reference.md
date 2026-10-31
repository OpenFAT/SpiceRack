# API reference

Signatures as the module reports them. In every element method the name and
nodes are positional; everything after `*` is keyword-only.

## Module `spicerack`

| Name | What it is |
| --- | --- |
| `Circuit(title)` | flat circuit; `str(circuit)` is its netlist |
| `Subcircuit(name, ports=None, **params)` | a block with ports |
| `Testbench(dut)` | stimulus and analyses around a `Subcircuit` |
| `CircuitSimulator` | what `Circuit.simulator()` returns; `CircuitSimulator.available_backends()` lists installed simulators |
| `ModelLibrary(path, corner=None, setup_includes=None, **backend_paths)` | a PDK model file and corner; `.name`, `.path`, `.corner` |
| `lint(netlist, backend=None)` | `{"errors": [...], "warnings": [...]}` |
| `compile_veriloga(source_or_path)` | compiles Verilog-A with OpenVAF, returns the `.osdi` path |
| `read_raw(path)` | every plot of a `.raw` file, as `RawData` |
| `unit` | unit constants, used as `10 @ u_kOhm` |

## Elements (`Circuit`, `Subcircuit`)

| Method | Signature |
| --- | --- |
| `R` | `(name, positive, negative, value, *, raw_spice=None, **params)` |
| `C`, `L` | `(name, positive, negative, value, **params)` |
| `K` | `(name, inductor1, inductor2, *, coupling)` |
| `V`, `I` | `(name, positive, negative, value, *, ac=None, ac_phase=None)` |
| `BV`, `BI` | `(name, positive, negative, *, expression)` |
| `E` | `(name, positive, negative, *, control_positive, control_negative, voltage_gain)` |
| `G` | `(name, positive, negative, *, control_positive, control_negative, transconductance)` |
| `F` | `(name, positive, negative, *, vsense, current_gain)` |
| `H` | `(name, positive, negative, *, vsense, transresistance)` |
| `POLY` | `(name, kind, positive, negative, controls, coefficients)`; `kind` is `"E"`, `"G"`, `"F"` or `"H"` |
| `D` | `(name, anode, cathode, *, model, **params)` |
| `Q` / `BJT` | `(name, collector, base, emitter, *, model, substrate=None, thermal=None, **params)` |
| `M` / `MOSFET` | `(name, drain, gate, source, bulk=None, *, model, extra_nodes=None, **params)` |
| `J`, `Z` | `(name, drain, gate, source, *, model, **params)` |
| `S` | `(name, positive, negative, *, control_positive, control_negative, model)` |
| `W` | `(name, positive, negative, *, vcontrol, model)` |
| `T` | `(name, input_positive, input_negative, output_positive, output_negative, *, Z0, TD)` |
| `X` | `(name, subcircuit_name, *nodes, **params)` |
| `instance` | `(subckt, name, *nodes, **params)`: like `X`, from the object |
| `A` | `(*, name, connections, model)`: XSPICE code model |

Waveform sources, each a `...VoltageSource` and a `...CurrentSource`, also on
`Testbench`; all take `dc=None, ac=None, ac_phase=None`:

| Method | Keyword arguments |
| --- | --- |
| `Sinusoidal` | `offset, amplitude, frequency, delay, damping_factor, phase` |
| `Pulse` | `initial_value, pulsed_value, delay, pulse_width, period, rise_time, fall_time` |
| `PieceWiseLinear` | `values=[(t, v), ...]` |
| `Exponential` | `initial_value, pulsed_value, rise_delay_time, rise_time_constant, fall_delay_time, fall_time_constant` |
| `SingleFrequencyFM` | `offset, amplitude, carrier_frequency, modulation_index, signal_frequency` |
| `AmplitudeModulated` | `offset, amplitude, modulating_frequency, carrier_frequency, signal_delay` |

## Directives and models (`Circuit`, `Subcircuit`)

| Method | Emits |
| --- | --- |
| `model(name, kind, **params)` | `.model` |
| `include(path)` | `.include` |
| `lib(path, section)` | `.lib` |
| `parameter(name, value)` | `.param` |
| `raw_spice(line)` | the line as written |
| `veriloga(source_or_path)` | a Verilog-A model, compiled per backend; returns the `.va` path |
| `osdi(path)` | a compiled OSDI model |
| `verilog(*, source, mode="simulate", instance_name, connections, pdk=None, liberty=None, spice_models=None)` | a digital Verilog module (co-simulated or synthesized) |
| `to_json()`, `save_json(path)`, `from_json(text)`, `load_json(path)` | the JSON form |
| `name`, `ports`, `gnd` | read-only; `gnd` is `"0"` |

`Circuit` only: `options(**kw)`, `temp(t)`, `node(name)`, `element(name)`,
`set_source_value(name, value)`, `subcircuit(subckt)`, `simulator(simulator=None)`.

## Testbench

| Method | Does |
| --- | --- |
| `V`, `I`, `R`, `C`, waveform sources | stimulus (no other elements) |
| `add_subcircuit(subckt)` | registers a child definition |
| `use_pdk(lib)` | attaches a `ModelLibrary` |
| `with_backend(name)` | pins the simulator |
| `netlist(backend="ngspice")`, `to_json()` | the deck, or its JSON |
| `check_backend(name)` | reasons it will not run on `name` |
| `global_nodes(*nodes)` | `.global` |
| `extra_line(line)` | a raw line in the bench |
| `add_operating_point()`, `add_dc(**sweeps)`, `add_ac(...)`, `add_transient(...)`, `add_noise(...)`, `add_tf(...)`, `add_fourier(f0, outputs)`, ... | queue an analysis card instead of running it |

## Settings (`Testbench`, `CircuitSimulator`)

`options(**kw)`, `save(*vectors)`, `measure(*parts)`, `initial_condition(**v)`,
`node_set(**v)`, `step(param, start, stop, step)`,
`step_sweep(param, start, stop, n, kind)`; attributes `temperature`,
`nominal_temperature`, and `save_currents` (`CircuitSimulator`).

## Analyses (`Testbench`, `CircuitSimulator`)

| Method | Returns |
| --- | --- |
| `operating_point()` | `OperatingPoint` |
| `dc(**{source: slice(start, stop, step)})` | `DcAnalysis` (`.sweep`, `.nodes`, `.branches`) |
| `ac(variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e9)` | `AcAnalysis` (`.frequency`) |
| `transient(step_time, end_time, start_time=None, max_time=None, use_initial_condition=False)` | `TransientAnalysis` (`.time`) |
| `noise(output_node, ref_node, src, variation="dec", points=10, start_frequency=1e3, stop_frequency=1e8, points_per_summary=None)` | `NoiseAnalysis` |
| `transfer_function(outvar, insrc)` / `tf` | `TransferFunctionAnalysis` |
| `dc_sensitivity(var)`, `ac_sensitivity(var, ...)` | `SensitivityAnalysis` |
| `polezero(node1, node2, node3, node4, tf_type, pz_type)` | `PoleZeroAnalysis` (`pz["pole(1)"]`) |
| `distortion(variation="dec", points=10, start_frequency=100.0, stop_frequency=1e8, f2overf1=None)` | `DistortionAnalysis` |
| `pss(fundamental_frequency, stabilization_time, observe_node, points_per_period=128, harmonics=10)` | `PssAnalysis` |
| `s_param(variation="dec", number_of_points=10, start_frequency=1e6, stop_frequency=1e10)` | `SParamAnalysis` |
| `harmonic_balance(fundamental_frequencies, num_harmonics=None)` | `HarmonicBalanceAnalysis` |
| `stability(probe, variation="dec", number_of_points=10, start_frequency=1.0, stop_frequency=1e10)` | `StabilityAnalysis` |
| `transient_noise(step_time, end_time)` | `TransientNoiseAnalysis` |
| `network_params(output_current, input_source, z_in=50.0, z_out=50.0, ...)` | `SParamAnalysis` (LTspice `.net`) |
| `spectre_sweep`, `spectre_montecarlo` | `RawData` (Spectre) |
| `spectre_pac`, `spectre_pnoise`, `spectre_pxf`, `spectre_pstb` | periodic small-signal (Spectre) |

## Results

Every result: `res["node"]` or `res.node` (a float for `OperatingPoint`, a list
otherwise) and `res.measures` (`.meas` values by name). AC-type results add
`magnitude(name)`, `magnitude_db(name)` and `phase(name)` (degrees). `RawData`
has `title`, `plot_name`, `is_complex`, `variable_names`, `measures` and
`complex(name)`.

## Units (`spicerack.unit`)

`u_V u_mV u_uV`, `u_A u_mA u_uA u_nA`, `u_Ohm u_kOhm u_MOhm` (`u_Ω u_kΩ u_MΩ`),
`u_F u_mF u_uF u_nF u_pF u_fF`, `u_H u_mH u_uH u_nH`, `u_Hz u_kHz u_MHz u_GHz`,
`u_s u_ms u_us u_ns u_ps`, `u_W u_mW u_uW`, `u_Degree`, plus `u_µ...` aliases.
A `UnitValue` has `.value` (SI float) and `.str_spice()` (`"10k"`).

Testbench and re-measuring functions: see [Testbenches](testbenches.md) and
[Re-measuring](remeasure.md).
