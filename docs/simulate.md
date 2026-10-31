# Simulating

Analyses run immediately and return a result. They exist on `Testbench` and on
the simulator a `Circuit` returns (`circuit.simulator()`).

```python
import spicerack as ps

rc = ps.Subcircuit("rc", ["vin", "vout"])
rc.R("1", "vin", "vout", 1e3)
rc.C("1", "vout", rc.gnd, 1e-9)

tb = ps.Testbench(rc)
tb.V("in", "vin", "0", 1.0, ac=1.0)

op   = tb.operating_point()
dc   = tb.dc(Vin=slice(0, 2, 0.1))                       # sweep source "in": name it V<name>
ac   = tb.ac(variation="dec", number_of_points=20, start_frequency=1e2, stop_frequency=1e8)
tran = tb.transient(step_time=1e-8, end_time=5e-6, max_time=1e-8)
nz   = tb.noise("vout", "0", "Vin", variation="dec", points=20, start_frequency=1e2, stop_frequency=1e8)
```

## Analyses

| Method | Returns |
| --- | --- |
| `operating_point()` | `OperatingPoint` |
| `dc(Vsrc=slice(start, stop, step), ...)` | `DcAnalysis`; two keywords give a nested sweep. `temp=slice(...)` sweeps temperature |
| `ac(variation, number_of_points, start_frequency, stop_frequency)` | `AcAnalysis` |
| `transient(step_time, end_time, start_time=None, max_time=None, use_initial_condition=False)` | `TransientAnalysis` |
| `noise(output_node, ref_node, src, variation, points, start_frequency, stop_frequency)` | `NoiseAnalysis` |
| `tf(outvar, insrc)` | `TransferFunctionAnalysis` (gain, input and output resistance) |
| `dc_sensitivity(var)`, `ac_sensitivity(var, ...)` | `SensitivityAnalysis` |
| `polezero(n1, n2, n3, n4, tf_type, pz_type)` | `PoleZeroAnalysis` |
| `distortion(...)` | `DistortionAnalysis` |
| `pss`, `s_param`, `harmonic_balance`, `stability`, `transient_noise`, `network_params` | RF and periodic analyses; each runs only where a backend supports it (see below) |
| `spectre_sweep`, `spectre_montecarlo`, `spectre_pac`, `spectre_pnoise`, `spectre_pxf`, `spectre_pstb` | Spectre only |

Set `max_time` on every transient you measure from: without it the simulator
picks its own grid, and edges and peaks move with it.

## Reading results

Every result reads the same way: index by node or branch name.

```python
print(op["vout"])                 # a float for an operating point
vout = tran["vout"]               # list[float] for a sweep, one per time point
t = tran.time                     # the axis: .time, .frequency or .sweep
mag = ac.magnitude_db("vout")     # AC results are complex:
ph = ac.phase("vout")             # degrees, wrapped to (-180, 180]
lin = ac.magnitude("vout")        # ac["vout"] alone is only the real part
density = nz["onoise_spectrum"]   # V/sqrt(Hz)
```

Branch currents are not saved unless you ask: `tb.save("I(Vin)")`, or
`sim.save_currents = True` on a `Circuit` simulator. `dc.nodes` and
`dc.branches` list what came back.

## Simulator settings

```python
tb.options(RELTOL=1e-4, ABSTOL=1e-12)         # .options
tb.temperature = 85.0                         # .temp
tb.nominal_temperature = 27.0                 # tnom
tb.initial_condition(vout=0.0)                # .ic (use with use_initial_condition=True)
tb.node_set(vout=0.5)                         # .nodeset, helps a stuck .op converge
tb.save("V(vout)", "I(Vin)")                  # only these vectors
tb.measure("tran", "vmax", "max", "v(vout)")  # .meas, lands in result.measures
res = tb.transient(step_time=1e-8, end_time=5e-6, max_time=1e-8)
print(res.measures["vmax"])
```

`tb.step(param, start, stop, step)` and `tb.step_sweep(param, start, stop, n, "dec")`
emit `.step`: LTspice, VACASK and Spectre run it, ngspice and ESPice do not.

## Netlists without simulating

```python
print(tb.netlist("ngspice"))      # also "ltspice", "vacask", "spectre", "espice", "hspice"
```

To write a deck with several analyses, queue them with the `add_*` twins
(`add_operating_point`, `add_ac`, `add_transient`, `add_noise`, `add_tf`,
`add_fourier`, ...) and print `tb.netlist(...)`. HSPICE is netlist-only: there
is no HSPICE runner.

## Choosing a simulator

```python
print(ps.CircuitSimulator.available_backends())   # what is installed
tb.with_backend("ngspice")                        # pin one for this bench
```

Without a choice, SpiceRack picks an installed backend that supports what the
circuit and the analysis need. `SPICERACK_BACKEND=vacask` in the environment
sets the default for every run. A flat `Circuit` takes it as
`circuit.simulator(simulator="vacask")`.

| Feature | ngspice | LTspice | VACASK | Spectre | ESPice |
| --- | --- | --- | --- | --- | --- |
| Verilog-A | yes (OpenVAF) | no | yes (OpenVAF) | yes | yes (compiles `.va` itself) |
| XSPICE `A` elements, control blocks | yes | no | no | no | no |
| `.meas` | yes | yes | no | yes | yes |
| `.step` sweeps | no | yes | yes | yes | no |
| Laplace sources | yes | yes | no | no | no |
| Digital Verilog | yes | no | no | yes | no |

The same design runs on several simulators with a loop:

```python
for backend in ps.CircuitSimulator.available_backends():
    if backend in ("ngspice", "vacask"):
        b = ps.Testbench(rc)
        b.V("in", "vin", "0", 1.0)
        b.with_backend(backend)
        print(backend, b.operating_point()["vout"])
```

## Linting

Catch problems before a run fails:

```python
print(tb.check_backend("ltspice"))          # list of reasons this bench will not run there
report = ps.lint(tb.netlist("ngspice"), backend="ngspice")
print(report["errors"], report["warnings"])
```

`lint` flags missing models, floating nodes and constructs the target cannot
take; each warning names the other backends it affects.

## Raw files

`ps.read_raw("out.raw")` reads a simulator's `.raw` file you already have
(ngspice, VACASK, ESPice, LTspice, Spectre nutbin; ASCII or binary) and returns
every plot in it. To recompute specs from one, see [Re-measuring](remeasure.md).
