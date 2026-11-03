"""
Example 20: PDK Hotswap

The same NMOS common-source amplifier simulated with the sky130 and gf180mcu
PDKs. The topology is identical; each PDK entry says how that PDK names and
sizes its NMOS and what its model library needs before ngspice can read it.

Needs PDK_ROOT pointing at a PDK root holding sky130A/ and gf180mcuD/
(`nix develop .#testing` sets one) and ngspice on PATH. A missing PDK or a
failed simulation exits non-zero.
"""
import os
import sys

import spicerack as ps
from spicerack.unit import u_kOhm

PDK_ROOT = os.environ.get("PDK_ROOT") or sys.exit(
    "PDK_ROOT is not set: point it at a directory holding sky130A/ and gf180mcuD/"
)
SKY130 = f"{PDK_ROOT}/sky130A/libs.tech/ngspice"
GF180 = f"{PDK_ROOT}/gf180mcuD/libs.tech/ngspice"

# Both PDKs' FETs are subcircuits, so the NMOS is an X card with W/L.
PDKS = {
    "sky130": {
        "lib": ps.ModelLibrary(f"{SKY130}/sky130.lib.spice", corner="tt"),
        # The mode sky130's own ngspice/spinit sets: read its libraries as HSPICE.
        "ngbehavior": "hsa",
        "nmos": "sky130_fd_pr__nfet_01v8",
        "size": {"W": 1, "L": 0.15},  # um: the library sets scale=1u
        "vdd": 1.8,
    },
    "gf180mcu": {
        # design.ngspice defines the switches every corner section reads.
        "lib": ps.ModelLibrary(f"{GF180}/sm141064.ngspice", corner="typical",
                               setup_includes=[f"{GF180}/design.ngspice"]),
        "ngbehavior": None,
        "nmos": "nfet_03v3",
        "size": {"W": 1e-6, "L": 0.28e-6},  # metres
        "vdd": 3.3,
    },
}


def build_cs_amp(nmos, size):
    """NMOS common-source amplifier. Topology is PDK-independent."""
    dut = ps.Subcircuit("cs_amp", ["vdd", "vin", "vout"])
    dut.R(name="drain", positive="vdd", negative="vout", value=5 @ u_kOhm)
    dut.X("n1", nmos, "vout", "vin", dut.gnd, dut.gnd, **size)
    return dut


for pdk_name, cfg in PDKS.items():
    lib = cfg["lib"]
    if not os.path.exists(lib.path):
        sys.exit(f"{pdk_name} PDK not installed: {lib.path} is missing")

    print(f"\n{'=' * 50}")
    print(f"PDK: {pdk_name}  (corner={lib.corner}, VDD={cfg['vdd']}V)")
    print(f"{'=' * 50}")

    dut = build_cs_amp(cfg["nmos"], cfg["size"])
    tb = ps.Testbench(dut)
    tb.use_pdk(lib)  # <-- PDK swap
    tb.ngbehavior = cfg["ngbehavior"]
    tb.V(name="dd", positive="vdd", negative=dut.gnd, value=cfg["vdd"])
    tb.V(name="gate", positive="vin", negative=dut.gnd, value=cfg["vdd"] / 2)
    tb.with_backend("ngspice")

    op = tb.operating_point()
    print(f"  V(vout) = {op['vout']:.6f} V")
    print(f"  V(vin)  = {op['vin']:.6f} V")

print("\nKey point: same topology, same testbench -- only the PDK entry changed.")
