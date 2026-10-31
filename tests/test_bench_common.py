"""Shared bench helpers. Oracle: a loop with a closed-form return ratio, run on ngspice."""
import numpy as np
import pytest

import spicerack as ps
from spicerack.testbenches._common import spot, tian_loop_gain

A0, R1, C1, RO, RIN, CIN = 1000.0, 1e3, 1.59154943e-7, 1e3, 1e3, 1e-9


def _loop():
    """A(s) = -A0/(1 + s R1 C1), buffered through Ro into Rin || Cin: T = A0/(1+sR1C1) * Zin/(Ro+Zin)."""
    d = ps.Subcircuit("loop", ["e", "f"])
    d.E(name="a", positive="n1", negative=d.gnd, control_positive="e", control_negative=d.gnd, voltage_gain=-A0)
    d.R(name="1", positive="n1", negative="n2", value=R1)
    d.C(name="1", positive="n2", negative=d.gnd, value=C1)
    d.E(name="b", positive="n3", negative=d.gnd, control_positive="n2", control_negative=d.gnd, voltage_gain=1.0)
    d.R(name="o", positive="n3", negative="f", value=RO)
    d.R(name="in", positive="e", negative=d.gnd, value=RIN)
    d.C(name="in", positive="e", negative=d.gnd, value=CIN)
    return d


def test_tian_recovers_the_return_ratio_through_finite_impedances():
    def build(v, i):
        tb = ps.Testbench(_loop())
        tb.V(name="prb", positive="e", negative="f", value=0.0, ac=v)
        tb.I(name="prb", positive="0", negative="e", value=0.0, ac=i)
        tb.save("V(e)", "I(Vprb)")
        return tb

    f, t = tian_loop_gain(build, dict(variation="dec", number_of_points=20, start_frequency=1, stop_frequency=1e8),
                          drive="e")
    s = 2j * np.pi * f
    zin = 1 / (1 / RIN + s * CIN)
    want = A0 / (1 + s * R1 * C1) * zin / (RO + zin)
    assert np.max(np.abs(t / want - 1)) < 1e-6  # a linear solve: only solver precision


@pytest.mark.parametrize("f, name", [(1e3, "x_1khz"), (100e3, "x_100khz"), (2.5e6, "x_2p5mhz"), (10, "x_10hz"),
                                     (0.1, "x_0p1hz"), (1e9, "x_1ghz")])
def test_spot_names(f, name):
    assert spot("x", f) == name
