"""Digital outputs read back from an analog transient: one owner for code reassembly."""

from __future__ import annotations

from typing import Any, Sequence

import numpy as np


def _codes(tran: Any, nodes: Sequence[str], times: Any, logic_high: float) -> np.ndarray:
    """Integer codes from bit ``nodes`` (LSB first) read at ``times``.

    Each bit is the node linearly interpolated at the read time, thresholded at
    half the logic swing. Read late in the conversion, where the bits are flat.
    """
    t = np.asarray(tran.time, float)
    bits = np.array([np.interp(times, t, np.asarray(tran[node], float)) > logic_high / 2.0 for node in nodes])
    return (bits.astype(np.int64) << np.arange(len(nodes))[:, None]).sum(axis=0)
