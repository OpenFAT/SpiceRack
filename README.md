# SpiceRack

[![Tests](https://github.com/OpenFAT/SpiceRack/actions/workflows/test.yaml/badge.svg?branch=main)](https://github.com/OpenFAT/SpiceRack/actions/workflows/test.yaml)
[![Docs](https://github.com/OpenFAT/SpiceRack/actions/workflows/docs.yaml/badge.svg?branch=main)](https://openfat.github.io/SpiceRack/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Build SPICE circuits in Python, run them on the simulator you have, and get the
results back as Python lists. One circuit description emits ngspice, LTspice,
VACASK, Spectre, ESPice and HSPICE decks, and about 60 ready-made testbenches
measure amplifiers, regulators, converters and PLLs for you.

```bash
pip install fast-spicerack       # imported as `spicerack`
```

You also need a simulator to run anything; [ngspice](https://ngspice.sourceforge.io/)
is the default. Without one, SpiceRack still writes netlists.

```python
import spicerack as ps

dut = ps.Subcircuit("divider", ["vin", "vout"])
dut.R("top", "vin", "vout", 2e3)
dut.R("bot", "vout", dut.gnd, 1e3)

tb = ps.Testbench(dut)
tb.V("supply", "vin", "0", 9.0)

print(tb.netlist("vacask"))             # the deck for any simulator
print(tb.operating_point()["vout"])     # 3.0, run on ngspice
```

A ready-made bench measures a whole spec at once:

```python
from spicerack.testbenches import step_response

rc = ps.Subcircuit("rc", ["vin", "vout"])
rc.R("1", "vin", "vout", 1e3)
rc.C("1", "vout", rc.gnd, 100e-12)

print(step_response(ps, rc, period=4e-6).measure()["settle_rise_0p1pct_s"])
```

## Documentation

**[openfat.github.io/SpiceRack](https://openfat.github.io/SpiceRack/)**, also readable here in [`docs/`](docs/index.md):

| Page | Covers |
| --- | --- |
| [Circuits](docs/circuits.md) | building circuits: elements, sources, units, models, hierarchy, JSON |
| [Simulating](docs/simulate.md) | analyses, results, simulator settings, choosing a backend, linting |
| [Models and PDKs](docs/models.md) | model libraries and corners, Verilog-A, digital Verilog |
| [Testbenches](docs/testbenches.md) | the bench catalog, corners and Monte Carlo, spec checks |
| [Re-measuring](docs/remeasure.md) | SNDR, settling, margins, noise and yield recomputed from `.raw` files |
| [Tools](docs/tools.md) | the `spice2rack` converter, the Rust crate, the JSON format |
| [API reference](docs/reference.md) | every class and signature |

[examples/](examples/) has 22 runnable scripts.

## Development

```bash
nix develop .#testing          # Rust, maturin, ngspice, OpenVAF, VACASK, PDKs
maturin develop && python3 -m pytest && cargo test
```

Pre-1.0: pin a version, as the API can change between minor releases.
MIT licensed, see [LICENSE](LICENSE).
