# Re-measuring

A pass a simulator reports is a claim. `spicerack.remeasure` recomputes the
deciding number from the waveforms in a `.raw` file, without the simulator's
`.meas` results and without running anything.

```python
import os, subprocess, tempfile
from spicerack import remeasure as rm

# a .raw file from any simulator run; here, ngspice on an RC noise deck
work = tempfile.mkdtemp()
deck = os.path.join(work, "rc.sp")
with open(deck, "w") as f:
    f.write("rc\nV1 in 0 DC 0 AC 1\nR1 in out 1k\nC1 out 0 1n\n.noise v(out) V1 dec 50 1 1e9\n.end\n")
subprocess.run(["ngspice", "-b", "-r", os.path.join(work, "rc.raw"), deck], check=True, capture_output=True)

raw = rm.read(os.path.join(work, "rc.raw"))     # every plot in the file
print([p.plot_name for p in raw])                # ['Noise Spectral Density Curves', 'Integrated Noise']
print(rm.noise_total(raw))                       # V rms, about sqrt(kT/C) = 2.0e-6
```

Each function finds its vector in the first plot that has it; pass `plot=` to
pick one.

## Spectrum: SNDR, SFDR, THD, ENOB

```python
# rm.sndr(raw, "v(out)", fs=5.12e6, fin=5.12e6 * 251 / 4096, n=4096)
# -> {"sndr_db", "sfdr_db", "enob", "thd_db", "signal_bin", "coherent", "signal_bins", "harmonic_bins"}
```

`sndr` resamples `n` points at rate `fs` (ending at the last time point, or
starting at `t0=`) and needs a coherent record: `fin * n / fs` an integer that
shares no factor with `n`. It refuses anything else unless you name a window
(`window="blackmanharris"`). Run the transient with a max step well below
`1/fs`, because interpolating a coarse time grid adds distortion.

## Time domain

| Call | Returns |
| --- | --- |
| `rm.settling(raw, "v(out)", final=0.9, tol=1e-3, t0=1e-6)` | seconds from `t0` until the signal stays within 0.1% of the step around `final` |
| `rm.settling(raw, "v(out)", final=0.9, abs_tol=1e-3, t0=1e-6)` | the same with a band in volts |
| `rm.slew(raw, "v(out)", lo=0.1, hi=0.9)` | `{"rising", "falling"}` in V/s between 10% and 90% of the swing |

`settling` returns `inf` when the record ends outside the band.

## Loop margins

`rm.loop_margins(raw, plot="Loop Stability Analysis")` reads a complex loop
gain (ESPice `.lstb` writes it as `loop_gain`) and returns `pm_deg`, `ugf_hz`,
`gm_db`, `pcf_hz` and `unity_crossings`. More than one unity crossing means a
conditionally stable or gain-boosted loop; read the first margin with care.

## Noise

`rm.noise_total(raw)` integrates `onoise_spectrum` over the swept band and
returns V rms. Each segment is integrated as a power law, so a sparse
log sweep stays accurate.

## Yield

```python
print(rm.yield_ci(passes=2300, n=2300, conf=0.95))   # (0.99840..., 1.0)
```

This is the exact (Clopper-Pearson) interval: 2300 clean Monte Carlo runs prove
a yield of at least 99.84%, not 100%.

## Checking a simulator's measures

ESPice writes its `.meas` results as JSON with `--measures=meas.json`.
`compare` lists every place they disagree with your recomputed numbers:

```python
measures = {"format": "espice-measures/1", "results": [
    {"name": "ugf", "analysis": "ac", "variant": {"kind": "nominal"}, "value": 30.3e6}]}
print(rm.compare(measures, {"ugf": 30.31e6}, rtol=1e-3))   # [] means they agree
```

A missing measure, a failed one and a value outside `rtol`/`atol` each give a
row with a `reason`. Pass the file path instead of a dict for a real run.
