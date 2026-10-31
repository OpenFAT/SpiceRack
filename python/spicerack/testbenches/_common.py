"""Helpers every bench family shares: complex AC vectors, Tian loop gain, metric names."""

from __future__ import annotations

from typing import Any, Callable

import numpy as np


def ac_complex(ac: Any, name: str) -> np.ndarray:
    """``name`` as complex numbers; ``ac[name]`` is only the real part."""
    return np.asarray(ac.magnitude(name), float) * np.exp(1j * np.radians(np.asarray(ac.phase(name), float)))


def tian(a: np.ndarray, b: np.ndarray, c: np.ndarray, d: np.ndarray) -> np.ndarray:
    """Return ratio T from Tian's two injections at a loop break (Tian et al. 2001).

    With ``Vprb e f`` in series (driven side ``e``) and ``Iprb 0 e`` into ``e``:
    the voltage run (v=1, i=0) gives ``b = -i(Vprb)``, ``d = v(e)``; the current
    run (v=0, i=1) gives ``a = -i(Vprb)``, ``c = v(e)``. T is positive at DC for
    negative feedback, exact for any impedance at the break.
    """
    return (2 * (a * d - b * c) - a + d) / (2 * (b * c - a * d) + a - d + 1)


def tian_loop_gain(build: Callable[[float, float], Any], ac_args: dict[str, Any],
                   drive: str, probe: str = "prb") -> tuple[np.ndarray, np.ndarray]:
    """``(f, T)`` from two AC runs of ``build(v_ac, i_ac)``, a Testbench holding
    ``V<probe> <drive> <other>`` and ``I<probe> 0 <drive>`` with those AC magnitudes."""
    v_run, i_run = build(1.0, 0.0).ac(**ac_args), build(0.0, 1.0).ac(**ac_args)
    branch = f"i(v{probe})"
    b, d = -ac_complex(v_run, branch), ac_complex(v_run, drive)
    a, c = -ac_complex(i_run, branch), ac_complex(i_run, drive)
    return np.asarray(v_run.frequency, float), tian(a, b, c, d)


def spot(prefix: str, f: float) -> str:
    """Metric name for a spot frequency: ``spot('psrr_db_at', 1e3) == 'psrr_db_at_1khz'``."""
    for scale, unit in ((1e9, "ghz"), (1e6, "mhz"), (1e3, "khz"), (1.0, "hz")):
        if f >= scale or scale == 1.0:
            return f"{prefix}_{f / scale:g}{unit}".replace(".", "p")
    raise AssertionError("unreachable")
