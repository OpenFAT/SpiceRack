#!/usr/bin/env bash
# Repro for ESPice: its HSPICE reader (--tokenizer=hspice) runs a sky130 `tt`
# NMOS to 5.1% more drain current than ngspice reading the same HSPICE text.
#
# The HSPICE text is sky130's combined library (libs.tech/combined, 194
# files) as SpiceRack writes it for HSPICE; ngspice (HSPICE mode, -D
# ngbehavior=hsa) reads it to the same current as the PDK's own files, bit
# for bit (tests/test_sections_pdk.rs, sky130_combined_library_converts_losslessly).
#
# Run inside `nix develop .#testing` from the SpiceRack checkout:
#   tests/espice/sky130_hspice_drain_current.sh [OUT_DIR]
# Measured (ESPice 0.3.0, ngspice 45): ESPice 1.8498e-4 A, ngspice 1.7608e-4 A;
# at l=0.5 7.0026e-5 against 6.8420e-5 (+2.3%).
set -euo pipefail
out=${1:-$(mktemp -d)}
SPICERACK_KEEP_HSPICE="$out/tree" cargo test -q --test test_sections_pdk sky130_combined >/dev/null
lib="$out/tree/libs.tech/combined/sky130.lib.spice"
cat > "$out/nmos.sp" <<DECK
sky130 tt nmos
.lib '$lib' tt
Xn d g 0 0 sky130_fd_pr__nfet_01v8 w=1 l=0.15
Vd d 0 1.8
Vg g 0 1.2
.op
.print op i(Vd)
.end
DECK
echo "ngspice (hsa):"
ngspice -D ngbehavior=hsa -b "$out/nmos.sp" 2>/dev/null | grep -i "vd#branch  "
echo "espice --tokenizer=hspice:"
espice --tokenizer=hspice "$out/nmos.sp" --rawfile "$out/nmos.raw" --format=ascii >/dev/null
grep -i -A6 "^Values" "$out/nmos.raw" || cat "$out/nmos.raw" | tail -8
echo "deck and tree: $out"
