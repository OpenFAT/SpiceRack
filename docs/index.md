# SpiceRack

Build SPICE circuits in Python, run them on the simulator you have, and get the
results back as Python lists. One circuit description emits ngspice, LTspice,
VACASK, Spectre, ESPice and HSPICE decks.

## Install

```bash
pip install fast-spicerack       # imported as `spicerack`
```

You also need a simulator on `$PATH` to run anything. Start with
[ngspice](https://ngspice.sourceforge.io/). Without one, SpiceRack still
writes netlists.

| Tool | What it adds |
| --- | --- |
| `ngspice` | The default backend; the only one with XSPICE, control blocks and digital Verilog |
| `vacask`, `spectre`, `ltspice`, `espice` | Other backends |
| `openvaf` | Verilog-A models on ngspice, VACASK and Spectre |
| `iverilog`, `yosys` | Digital Verilog: co-simulation, or synthesis to gates |

From source: `pip install maturin && maturin develop` in a clone (needs Rust).
With Nix, `nix develop .#testing` gives the build tools plus ngspice, OpenVAF,
VACASK and the sky130/gf180 PDKs.

## Quick start

A voltage divider, solved for its operating point:

```python
import spicerack as ps

dut = ps.Subcircuit("divider", ["vin", "vout"])
dut.R("top", "vin", "vout", 2e3)
dut.R("bot", "vout", dut.gnd, 1e3)

tb = ps.Testbench(dut)
tb.V("supply", "vin", "0", 9.0)

print(tb.netlist("ngspice"))           # the deck, no simulator needed
print(tb.operating_point()["vout"])     # 3.0
```

An RC low-pass, swept in frequency:

```python
rc = ps.Subcircuit("rc", ["vin", "vout"])
rc.R("1", "vin", "vout", 1e3)
rc.C("1", "vout", rc.gnd, 1e-6)

tb = ps.Testbench(rc)
tb.V("in", "vin", "0", 0.0, ac=1.0)
ac = tb.ac(variation="dec", number_of_points=50, start_frequency=1, stop_frequency=1e5)

gain_db = ac.magnitude_db("vout")       # list[float], one per ac.frequency point
```

## What is in here

| Page | Covers |
| --- | --- |
| [Circuits](circuits.md) | `Circuit`, `Subcircuit`, `Testbench`, every element and source, units, models, saving to JSON |
| [Simulating](simulate.md) | Analyses, reading results, simulator options, choosing a backend, linting |
| [Models and PDKs](models.md) | Model libraries and corners, Verilog-A, digital Verilog |
| [Testbenches](testbenches.md) | About 60 ready-made benches (amplifiers, regulators, converters, PLLs...), corners and Monte Carlo |
| [Re-measuring](remeasure.md) | Recomputing SNDR, settling, margins, noise and yield from a `.raw` file |
| [Tools](tools.md) | The `spice2rack` converter, the Rust crate, the JSON circuit format |
| [API reference](reference.md) | Every class, method and signature on one page |

The [examples/](https://github.com/OpenFAT/SpiceRack/tree/main/examples) folder
has 22 runnable scripts, from a voltage divider to the testbench library.

SpiceRack is pre-1.0: pin a version, since the API can change between minor
releases. MIT licensed.
