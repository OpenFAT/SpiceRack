"""
Tests that all Python examples execute without errors.

Each example generates a netlist and prints it. We import and run them
to verify no crashes in the circuit builder or unit system.

Run: maturin develop && pytest tests/test_python_examples.py -v
"""
import pytest
import re
import subprocess
import sys
import os
from pathlib import Path

EXAMPLES_DIR = Path(__file__).parent.parent / "examples"
REPO_ROOT = EXAMPLES_DIR.parent


def run_example(filename, timeout=30):
    """Run an example script as subprocess, return (returncode, stdout, stderr)."""
    import spicerack  # noqa: F401  unsupported fails, never skips: an unbuilt extension is an error
    script = EXAMPLES_DIR / filename
    assert script.exists(), f"{filename} not found"
    env = os.environ.copy()
    paths = [str(REPO_ROOT / "python"), str(REPO_ROOT), env.get("PYTHONPATH", "")]
    env["PYTHONPATH"] = os.pathsep.join(path for path in paths if path)
    result = subprocess.run(
        [sys.executable, str(script)],
        capture_output=True, text=True, timeout=timeout, env=env,
    )
    return result


class TestExamples:
    def test_01_voltage_divider(self):
        r = run_example("01_voltage_divider.py")
        assert r.returncode == 0, r.stderr
        assert "Match: yes" in r.stdout

    def test_02_rc_lowpass(self):
        r = run_example("02_rc_lowpass.py")
        assert r.returncode == 0, r.stderr
        assert "f_3dB expected" in r.stdout
        assert "tau = R*C" in r.stdout

    def test_03_rlc_bandpass(self):
        r = run_example("03_rlc_bandpass.py")
        assert r.returncode == 0, r.stderr
        assert "Expected resonance" in r.stdout

    def test_04_bjt_amplifier(self):
        r = run_example("04_bjt_amplifier.py")
        assert r.returncode == 0, r.stderr
        assert "Approximate bias" in r.stdout

    def test_05_cmos_inverter(self):
        r = run_example("05_cmos_inverter.py")
        assert r.returncode == 0, r.stderr
        assert "Vout range" in r.stdout

    def test_06_opamp_inverting(self):
        r = run_example("06_opamp_inverting.py")
        assert r.returncode == 0, r.stderr
        assert "Match: yes" in r.stdout

    def test_07_diode_rectifier(self):
        r = run_example("07_diode_rectifier.py")
        assert r.returncode == 0, r.stderr
        assert "Expected" in r.stdout

    def test_08_differential_pair(self):
        r = run_example("08_differential_pair.py")
        assert r.returncode == 0, r.stderr
        assert "Vcm_out" in r.stdout

    def test_09_controlled_sources(self):
        r = run_example("09_controlled_sources.py")
        assert r.returncode == 0, r.stderr
        assert "VCVS" in r.stdout
        assert "VCCS" in r.stdout
        assert "CCCS" in r.stdout
        assert "CCVS" in r.stdout

    def test_10_subcircuit(self):
        r = run_example("10_subcircuit.py")
        assert r.returncode == 0, r.stderr
        assert "V(output)" in r.stdout

    def test_11_jfet_amplifier(self):
        r = run_example("11_jfet_amplifier.py")
        assert r.returncode == 0, r.stderr
        assert "Vdrain" in r.stdout

    def test_12_transmission_line(self):
        r = run_example("12_transmission_line.py")
        assert r.returncode == 0, r.stderr
        assert "Reflection coefficient" in r.stdout

    def test_13_switches(self):
        r = run_example("13_switches.py")
        assert r.returncode == 0, r.stderr
        assert "S: ON" in r.stdout
        assert "W: ON" in r.stdout

    def test_14_pwl_dac(self):
        r = run_example("14_pwl_dac.py")
        assert r.returncode == 0, r.stderr
        assert "LSB" in r.stdout

    def test_15_linting(self):
        r = run_example("15_linting.py")
        assert r.returncode == 0, r.stderr

    def test_16_simulator_config(self):
        r = run_example("16_simulator_config.py")
        assert r.returncode == 0, r.stderr
        assert "Testbench configured" in r.stdout

    def test_17_veriloga_inline(self):
        r = run_example("17_veriloga_inline.py")
        assert r.returncode == 0, r.stderr

    def test_18_verilog_digital(self):
        r = run_example("18_verilog_digital.py")
        assert r.returncode == 0, r.stderr

    def test_19_backend_hotswap(self):
        r = run_example("19_backend_hotswap.py")
        assert r.returncode == 0, r.stderr
        assert "Backend: ngspice" in r.stdout

    # Goldens: V(vout) from ngspice-45 and ESPice 0.3.0 in `nix develop
    # .#testing`, on the PDKs pinned in nix/pdks.nix (sky130 8afc834,
    # gf180mcu 1e35547), recorded 2026-11-02. Tolerance 1e-3 relative:
    # ngspice's default RELTOL, the accuracy an .op converges to, so another
    # simulator build may land anywhere inside it. Each example loads sky130's
    # whole tt corner in ngspice (~40 s); hence the longer timeout.

    def test_20_pdk_hotswap(self):
        r = run_example("20_pdk_hotswap.py", timeout=300)
        assert r.returncode == 0, r.stderr
        vout = {pdk: float(v) for pdk, v in re.findall(r"PDK: (\w+).*?V\(vout\) = ([\d.]+) V", r.stdout, re.S)}
        assert vout == pytest.approx({"sky130": 1.576323, "gf180mcu": 2.507821}, rel=1e-3)
        # The NMOS conducts: its 5k drain resistor drops more than 0.1 V.
        assert 0 < vout["sky130"] < 1.8 - 0.1
        assert 0 < vout["gf180mcu"] < 3.3 - 0.1

    def test_21_combined_hotswap(self):
        r = run_example("21_combined_hotswap.py", timeout=300)
        assert r.returncode == 0, r.stderr
        vout = {(pdk, sim): float(v) for pdk, sim, v in re.findall(r"--- (\w+) / (\w+) ---\s+V\(vout\) = ([\d.]+) V", r.stdout)}
        assert vout == pytest.approx({
            ("sky130", "ngspice"): 0.146308, ("sky130", "espice"): 0.146308,
            ("gf180mcu", "ngspice"): 0.295838,
        }, rel=1e-3)
        # Two simulators, one deck: ESPice is an independent oracle for ngspice.
        assert vout[("sky130", "espice")] == pytest.approx(vout[("sky130", "ngspice")], rel=1e-3)
        assert 0 < vout[("sky130", "ngspice")] < 1.8
        assert 0 < vout[("gf180mcu", "ngspice")] < 3.3

    def test_22_design_testbenches(self):
        r = run_example("22_design_testbenches.py")
        assert r.returncode == 0, r.stderr
        assert "Validated 12 reusable design testbench recipes" in r.stdout
