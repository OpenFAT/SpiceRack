# Circuits

## Three objects

| Object | Use it for | Create with |
| --- | --- | --- |
| `Subcircuit` | A design block with ports: the thing you test | `ps.Subcircuit("name", ["port", ...])` |
| `Testbench` | Stimulus and analyses around one `Subcircuit` | `ps.Testbench(dut)` |
| `Circuit` | A standalone, flat circuit (the PySpice style) | `ps.Circuit("title")` |

Put devices in the `Subcircuit` and sources in the `Testbench`. A `Testbench`
only takes `V`, `I`, `R`, `C` and the waveform sources; everything else
(`L`, `E`, `M`, `model`, `include`, ...) lives on `Subcircuit` and `Circuit`.

```python
import spicerack as ps

dut = ps.Subcircuit("amp", ["vin", "vout", "vdd"])
dut.R("load", "vdd", "vout", 10e3)
dut.M("1", "vout", "vin", dut.gnd, dut.gnd, model="nmos1", W=10e-6, L=1e-6)
dut.model("nmos1", "NMOS", LEVEL=1, VTO=0.5, KP=200e-6)

tb = ps.Testbench(dut)
tb.V("dd", "vdd", "0", 1.8)
tb.V("in", "vin", "0", 0.8, ac=1.0)
```

The flat style builds everything in one `Circuit` and gets a simulator from it:

```python
c = ps.Circuit("divider")
c.V("in", "vin", c.gnd, 10.0)
c.R("1", "vin", "vout", 1e3)
c.R("2", "vout", c.gnd, 1e3)
op = c.simulator().operating_point()
print(op["vout"])   # 5.0
print(c)            # the netlist
```

Ground is `dut.gnd` (node `"0"`). In a `Testbench`, write `"0"`.

## Values and units

A value is a float in SI units, a unit value, or a SPICE expression string:

```python
from spicerack.unit import u_kOhm, u_pF

r = 4.7 @ u_kOhm
print(r.value, r.str_spice())   # 4700.0 4.7k

dut.parameter("rl", "10k")
dut.R("x", "vout", dut.gnd, "rl*2")
dut.C("c1", "vout", dut.gnd, 2 @ u_pF)
```

Unit names follow `u_<prefix><unit>`: `u_V`, `u_mA`, `u_kOhm`, `u_pF`, `u_uH`,
`u_MHz`, `u_ns`, `u_mW`, `u_Degree`. `u_µ...` and `u_Ω` aliases exist.

## Elements

Every element takes its name and nodes first; the rest are keywords.

| Method | Element | Arguments after `name, nodes...` |
| --- | --- | --- |
| `R`, `C`, `L` | passives | `value`, then instance parameters (`tc1=...`) |
| `K` | inductor coupling | `inductor1, inductor2, coupling=` |
| `V`, `I` | DC sources | `value, ac=, ac_phase=` |
| `E`, `G` | voltage-controlled sources | `control_positive=, control_negative=, voltage_gain=` / `transconductance=` |
| `F`, `H` | current-controlled sources | `vsense=` (a V source name), `current_gain=` / `transresistance=` |
| `POLY` | polynomial E/G/F/H | `kind, positive, negative, controls, coefficients` |
| `BV`, `BI` | behavioural sources | `expression="..."` |
| `D` | diode | `anode, cathode, model=` |
| `Q` (`BJT`) | bipolar | `collector, base, emitter, model=` |
| `M` (`MOSFET`) | MOSFET | `drain, gate, source, bulk, model=`, then `W=`, `L=`, ... |
| `J`, `Z` | JFET, MESFET | `drain, gate, source, model=` |
| `S`, `W` | switches | `control_positive=, control_negative=, model=` / `vcontrol=, model=` |
| `T` | transmission line | four nodes, `Z0=, TD=` |
| `X` | subcircuit instance by name | `"subckt_name", *nodes, **params` |
| `A` | XSPICE code model | `name=, connections=[...], model=` (all keywords) |

```python
dut.E("buf", "vbuf", dut.gnd, control_positive="vout", control_negative=dut.gnd, voltage_gain=1.0)
dut.BI("leak", "vout", dut.gnd, expression="1e-9*V(vout)")
dut.V("sense", "vout", "vload", 0.0)                  # a 0 V source to sense current
dut.F("mirror", "vdd", "vcopy", vsense="Vsense", current_gain=2.0)
```

## Waveform sources

Each comes as `...VoltageSource` and `...CurrentSource`, on all three objects:

| Method | Key arguments |
| --- | --- |
| `SinusoidalVoltageSource` | `offset, amplitude, frequency, delay, damping_factor, phase` |
| `PulseVoltageSource` | `initial_value, pulsed_value, delay, rise_time, fall_time, pulse_width, period` |
| `PieceWiseLinearVoltageSource` | `values=[(t, v), ...]` |
| `ExponentialVoltageSource` | `initial_value, pulsed_value, rise_delay_time, rise_time_constant, fall_delay_time, fall_time_constant` |
| `SingleFrequencyFMVoltageSource` | `offset, amplitude, carrier_frequency, modulation_index, signal_frequency` |
| `AmplitudeModulatedVoltageSource` | `offset, amplitude, modulating_frequency, carrier_frequency, signal_delay` |

All take `dc=` (the DC value for `.op`; default: the waveform at t=0), `ac=` and `ac_phase=`.

```python
tb.PulseVoltageSource("clk", "vclk", "0", initial_value=0.0, pulsed_value=1.8,
                      rise_time=1e-9, fall_time=1e-9, pulse_width=24e-9, period=50e-9)
tb.PieceWiseLinearVoltageSource("ramp", "vramp", "0", values=[(0, 0.0), (1e-6, 1.8)])
```

## Models, libraries, raw lines

```python
dut.model("D1N4148", "D", IS=2.52e-9, N=1.752)    # .model
dut.include("models.lib")                         # .include
dut.lib("sky130.lib.spice", "tt")                 # .lib file section
dut.parameter("vbias", "0.9")                     # .param (value is a string)
dut.raw_spice("* anything the builder does not cover")
```

`Circuit` also has `options(**kw)`, `temp(t)`, `node(name)`, `element(name)`
and `set_source_value(name, value)`. For model libraries with corners, see
[Models and PDKs](models.md).

## Hierarchy

Build a child `Subcircuit`, place it with `instance` (or `X` by name), and
register its definition once with `add_subcircuit` (`Circuit.subcircuit` on a
`Circuit`):

```python
inv = ps.Subcircuit("half", ["a", "y"])
inv.R("1", "a", "y", 1e3)
inv.R("2", "y", inv.gnd, 1e3)

top = ps.Subcircuit("top", ["in", "out"])
top.instance(inv, "u1", "in", "mid")
top.X("u2", "half", "mid", "out")                 # by name, e.g. a PDK device
tb = ps.Testbench(top)
tb.add_subcircuit(inv)
tb.V("in", "in", "0", 1.0)
print(tb.operating_point()["out"])                # 0.2 (u2 loads the first divider)
```

Keyword arguments to `X` and `instance` become instance parameters
(`dut.X("m1", "sky130_fd_pr__nfet_01v8", "d", "g", "s", "b", W=1, L=0.15)`).
`dut.name` and `dut.ports` read a block back.

## Saving circuits

Circuits round-trip through JSON, the same format any tool can write
(schema: [`schema/circuit-ir.schema.json`](https://github.com/OpenFAT/SpiceRack/blob/main/schema/circuit-ir.schema.json)):

```python
import tempfile, os

path = os.path.join(tempfile.mkdtemp(), "top.json")
top.save_json(path)
again = ps.Subcircuit.load_json(path)
print(again.name, again.ports)                    # top ['in', 'out']
text = top.to_json()                              # or keep it as a string
same = ps.Subcircuit.from_json(text)
```
