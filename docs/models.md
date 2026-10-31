# Models and PDKs

## Model libraries and corners

A `ModelLibrary` names a PDK's model file and corner. Attach it to a bench; the
netlist gets the right `.lib` line for whichever simulator runs it.

<!-- not run: needs the sky130 PDK under $PDK_ROOT -->
```python
import os
import spicerack as ps

lib = ps.ModelLibrary(f"{os.environ['PDK_ROOT']}/sky130A/libs.tech/ngspice/sky130.lib.spice", corner="tt")

dut = ps.Subcircuit("cs", ["vdd", "vin", "vout"])
dut.R("load", "vdd", "vout", 20e3)
dut.X("m1", "sky130_fd_pr__nfet_01v8", "vout", "vin", dut.gnd, dut.gnd, W=2, L=0.15)

tb = ps.Testbench(dut)
tb.use_pdk(lib)
tb.V("dd", "vdd", "0", 1.8)
tb.V("in", "vin", "0", 0.7)
print(tb.operating_point()["vout"])
```

`ModelLibrary(path, corner=None, setup_includes=None, **backend_paths)`: pass
`spectre="..."`, `vacask="..."` and so on when each simulator needs its own
model file. Swapping PDKs is swapping the library and the device names; the
topology stays.

For every corner, temperature and supply at once, build the list with
`pdk_corners` and run it with `run_corners` (see
[Testbenches](testbenches.md#corners-and-monte-carlo)):

<!-- not run: needs the sky130 PDK under $PDK_ROOT -->
```python
from spicerack.testbenches import pdk_corners

corners = pdk_corners(lib.path, ["tt", "ss", "ff"], temperatures=[-40, 27, 125], vdds=[1.62, 1.98])
```

## Verilog-A

`veriloga()` takes a `.va` path or the source text. SpiceRack compiles it with
OpenVAF for ngspice and VACASK, includes it for Spectre, and hands the source
to ESPice. Place the instance with its model name:

```python
import spicerack as ps

VRES = """
`include "disciplines.vams"
module vres(a, b);
    inout a, b; electrical a, b;
    parameter real r = 1000.0;
    analog I(a, b) <+ V(a, b) / r;
endmodule
"""

dut = ps.Subcircuit("vdiv", ["vdd", "out"])
dut.veriloga(VRES)                           # returns the .va path
dut.model("vres_3k", "vres", r=3e3)          # ngspice places an OSDI device through a .model
dut.R("top", "vdd", "out", 1e3)
dut.raw_spice("N1 out 0 vres_3k")            # the instance card

tb = ps.Testbench(dut)
tb.V("dd", "vdd", "0", 1.0)
tb.with_backend("ngspice")
print(tb.operating_point()["out"])           # 0.75: 1 kOhm over the 3 kOhm model
```

Parameters go on the `.model` card. A parameter the module marks
`(* type="instance" *)` can instead go on the instance line
(`N1 out 0 vres_3k r=3k`); ngspice refuses any other parameter there.
Needs `openvaf` on `$PATH` (except on ESPice), and an ngspice whose OSDI
version matches it. `dut.osdi("model.osdi")` loads a binary you compiled
yourself; `ps.compile_veriloga("model.va")` compiles one and returns its path.

## Digital Verilog

Drop a Verilog module into an analog circuit. `mode="simulate"` co-simulates it
with Icarus through ngspice's XSPICE `d_cosim`; `mode="synthesize"` runs Yosys
against a Liberty file and emits gate-level subcircuits instead.

<!-- not run: needs ngspice with XSPICE plus iverilog -->
```python
dut = ps.Subcircuit("mixed", ["clk", "rst", "q"])
dut.verilog(
    source="counter.v",
    mode="simulate",
    instance_name="u_counter",
    connections={"clk": "clk", "rst": "rst", "count": ["q0", "q1", "q2", "q3"]},
)
```

A connection value is a net name or a list of nets (a bus). For synthesis add
`liberty="cells.lib"` and `spice_models="cells.spice"` (or `pdk=`). The bridges
between analog and digital nets are inserted for you; to place them by hand,
use `model(..., "adc_bridge", ...)` and `A(name=..., connections=[...], model=...)`
as in example 18.
