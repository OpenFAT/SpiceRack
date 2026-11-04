"""
Example 21: Combined Backend + PDK Hotswap

The same CMOS inverter across a matrix of PDKs and backends: one circuit, any
PDK, any backend that reads the PDK's model files.

ModelLibrary accepts per-backend paths as keyword arguments, and the deck
picks the right file. The open PDKs ship ngspice model files only, so the
matrix runs the backends that read those: ngspice, and ESPice where it reads
the PDK. VACASK and Spectre would need a `vacask=`/`spectre=` path to the
PDK's own files.

Needs PDK_ROOT pointing at a PDK root holding sky130A/ and gf180mcuD/
(`nix develop .#testing` sets one), ngspice and espice on PATH. A missing PDK
or a failed simulation exits non-zero.
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

# Both PDKs' FETs are subcircuits, so each FET is an X card with W/L.
PDKS = {
    "sky130": {
        "lib": ps.ModelLibrary(f"{SKY130}/sky130.lib.spice", corner="tt"),
        # The mode sky130's own ngspice/spinit sets; ESPice needs none.
        "ngbehavior": "hsa",
        "nmos": ("sky130_fd_pr__nfet_01v8", {"W": 1, "L": 0.15}),  # um: scale=1u
        "pmos": ("sky130_fd_pr__pfet_01v8", {"W": 2, "L": 0.15}),
        "vdd": 1.8,
        "backends": ["ngspice", "espice"],
    },
    "gf180mcu": {
        # design.ngspice defines the switches every corner section reads.
        "lib": ps.ModelLibrary(f"{GF180}/sm141064.ngspice", corner="typical",
                               setup_includes=[f"{GF180}/design.ngspice"]),
        "ngbehavior": None,
        "nmos": ("nfet_03v3", {"W": 1e-6, "L": 0.28e-6}),  # metres
        "pmos": ("pfet_03v3", {"W": 2e-6, "L": 0.28e-6}),
        "vdd": 3.3,
        # ESPice 0.3.0 refuses the BSIM4 instance parameter `mulu0` that the
        # gf180 FET subcircuits set.
        "backends": ["ngspice"],
    },
}


def build_inverter(nmos, pmos):
    """CMOS inverter -- topology is universal across PDKs."""
    dut = ps.Subcircuit("inverter", ["vdd", "vin", "vout"])
    dut.X("p1", pmos[0], "vout", "vin", "vdd", "vdd", **pmos[1])
    dut.X("n1", nmos[0], "vout", "vin", dut.gnd, dut.gnd, **nmos[1])
    return dut


print("Combined Backend + PDK Hotswap Matrix")
print("=" * 60)

for pdk_name, cfg in PDKS.items():
    lib = cfg["lib"]
    if not os.path.exists(lib.path):
        sys.exit(f"{pdk_name} PDK not installed: {lib.path} is missing")

    for backend in cfg["backends"]:
        print(f"\n--- {pdk_name} / {backend} ---")

        dut = build_inverter(cfg["nmos"], cfg["pmos"])
        tb = ps.Testbench(dut)
        tb.use_pdk(lib)
        tb.ngbehavior = cfg["ngbehavior"]
        tb.V(name="dd", positive="vdd", negative=dut.gnd, value=cfg["vdd"])
        tb.V(name="in", positive="vin", negative=dut.gnd, value=cfg["vdd"] / 2)
        tb.R(name="load", positive="vout", negative=dut.gnd, value=10 @ u_kOhm)
        tb.with_backend(backend)

        op = tb.operating_point()
        print(f"  V(vout) = {op['vout']:.6f} V")

print("\nKey point: one build_inverter(), swapped via ModelLibrary + with_backend().")
