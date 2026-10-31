"""Waveform metrics, validation, corner and Monte Carlo helpers for design benches.

Waveforms are numpy columns; runs are rows (one dict of metrics per run).
"""

from __future__ import annotations

import math
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Mapping, NamedTuple, Sequence

import numpy as np


def _testbench(obj: Any) -> Any:
    return getattr(obj, "testbench", obj)


# ── waveform primitives ──

def _interp_at(x: Sequence[float], y: Sequence[float], x_target: float) -> float:
    """y at ``x_target``, linear, clamped to the end values outside the record."""
    if len(x) == 0 or len(y) == 0:
        raise ValueError("cannot interpolate an empty waveform")
    n = min(len(x), len(y))
    return float(np.interp(x_target, np.asarray(x, float)[:n], np.asarray(y, float)[:n]))


def crossings(
    x_values: Sequence[float],
    y_values: Sequence[float],
    threshold: float,
    rising: bool | None = None,
    log_x: bool = False,
) -> list[float]:
    """Every x where y crosses ``threshold``, linearly interpolated.

    ``rising=None`` returns both directions. ``log_x=True`` interpolates in
    log10(x), which matters on a decade-spaced frequency sweep: linear
    interpolation between decade points misplaces a -3 dB corner by ~1% at
    10 points/decade.
    """
    n = min(len(x_values), len(y_values))
    x = np.asarray(x_values, float)[:n]
    y = np.asarray(y_values, float)[:n]
    x0, x1, y0, y1 = x[:-1], x[1:], y[:-1], y[1:]
    up = (y0 <= threshold) & (threshold <= y1)
    down = (y0 >= threshold) & (threshold >= y1)
    hit = up if rising is True else down if rising is False else up | down
    x0, x1, y0, y1 = x0[hit], x1[hit], y0[hit], y1[hit]
    flat = y1 == y0
    with np.errstate(divide="ignore", invalid="ignore"):
        frac = np.where(flat, 1.0, (threshold - y0) / (y1 - y0))
        lin = x0 + (x1 - x0) * frac
        if log_x:
            lo, hi = np.log10(x0), np.log10(x1)
            lin = np.where((x0 > 0) & (x1 > 0), 10.0 ** (lo + (hi - lo) * frac), lin)
    return np.where(flat, x1, lin).tolist()


# ── AC-domain extractors ──
#
# These need magnitude and phase, which `ac[node]` does not provide (it is the
# real part). They go through `ac.magnitude_db()` / `ac.phase()`.

#: The half-power point, 10*log10(2) = 3.0103 dB -- not 3.000 dB.
#: Using a round 3.0 puts the corner of a single-pole response 0.23% low,
#: a systematic error that does not shrink with sweep density.
HALF_POWER_DB = 10.0 * math.log10(2.0)


def ac_gain_db(ac: Any, node: str, at_hz: float) -> float:
    """Gain in dB at one frequency (log-interpolated between sweep points)."""
    return _interp_at(np.log10(ac.frequency), ac.magnitude_db(node), math.log10(at_hz))


def ac_passband_db(ac: Any, node: str) -> float:
    """Reference gain: the passband maximum.

    Referencing to the *peak* rather than to DC is what makes this correct for
    Chebyshev and bandpass responses, where the DC value is not the passband.
    """
    return float(np.max(ac.magnitude_db(node)))


def ac_bandwidth_hz(ac: Any, node: str, drop_db: float = HALF_POWER_DB,
                    reference_db: float | None = None) -> float:
    """Frequency where the response falls ``drop_db`` below the passband.

    Defaults to the half-power point (3.0103 dB), which is what "-3 dB
    bandwidth" means. Pass ``drop_db=3.0`` for the literal-3 dB convention.
    """
    freq, db = np.asarray(ac.frequency, float), np.asarray(ac.magnitude_db(node), float)
    peak = int(np.argmax(db)) if reference_db is None else 0
    ref = db[peak] if reference_db is None else reference_db
    hits = crossings(freq[peak:], db[peak:], ref - drop_db, rising=False, log_x=True)
    if not hits:
        raise ValueError(
            f"'{node}' never falls {drop_db} dB below its passband "
            f"({ref:.3f} dB) within the swept range"
        )
    return hits[0]


def ac_unity_gain_hz(ac: Any, node: str) -> float:
    """Frequency where the magnitude crosses 0 dB going down."""
    hits = crossings(ac.frequency, ac.magnitude_db(node), 0.0, rising=False, log_x=True)
    if not hits:
        raise ValueError(f"'{node}' never crosses 0 dB within the swept range")
    return hits[0]


def ac_phase_margin_deg(ac: Any, node: str) -> float:
    """Phase margin = 180 + phase at the unity-gain crossing.

    Only meaningful on an open-loop response. On a closed-loop response this
    number is not a stability margin.
    """
    f_unity = ac_unity_gain_hz(ac, node)
    # `ac.phase` wraps to (-180, 180]: interpolate the unwrapped phase, then
    # wrap the margin, so an unstable loop at -200 deg reads -20, not 340.
    phase = np.degrees(np.unwrap(np.radians(np.asarray(ac.phase(node), float))))
    pm = 180.0 + _interp_at(np.log10(ac.frequency), phase, math.log10(f_unity))
    return (pm + 180.0) % 360.0 - 180.0


# ── metric specs and validation ──

@dataclass(frozen=True)
class MetricSpec:
    name: str
    source: str
    reducer: str = "last"
    at: float | None = None
    threshold: float | None = None
    rising: bool = True


def _signal(result: Any, name: str) -> np.ndarray:
    if not isinstance(result, Mapping):
        if name in getattr(result, "measures", ()):
            result = result.measures
        elif hasattr(result, name):
            return np.atleast_1d(np.asarray(getattr(result, name), float))
    return np.atleast_1d(np.asarray(result[name], float))


_REDUCERS: dict[str, Callable[[np.ndarray], float]] = {
    "first": lambda v: v[0], "initial": lambda v: v[0],
    "last": lambda v: v[-1], "final": lambda v: v[-1],
    "min": np.min, "max": np.max, "mean": np.mean,
    "abs_max": lambda v: np.max(np.abs(v)),
    "peak_to_peak": np.ptp, "pp": np.ptp,
}


def extract_metrics(result: Any, specs: Sequence[MetricSpec]) -> dict[str, float]:
    metrics: dict[str, float] = {}
    for spec in specs:
        values = _signal(result, spec.source)
        reducer = spec.reducer.lower()
        if reducer in _REDUCERS:
            metrics[spec.name] = float(_REDUCERS[reducer](values))
        elif reducer == "at":
            if spec.at is None:
                raise ValueError(f"MetricSpec '{spec.name}' requires at=")
            has_time = "time" in result if isinstance(result, Mapping) else hasattr(result, "time")
            axis = _signal(result, "time" if has_time else "frequency")
            metrics[spec.name] = _interp_at(axis, values, spec.at)
        elif reducer == "crossing_time":
            if spec.threshold is None:
                raise ValueError(f"MetricSpec '{spec.name}' requires threshold=")
            hits = crossings(_signal(result, "time"), values, spec.threshold, rising=spec.rising)
            if not hits:
                raise ValueError(f"waveform never crossed {spec.threshold}")
            metrics[spec.name] = hits[0]
        else:
            raise ValueError(f"unknown metric reducer '{spec.reducer}'")
    return metrics


class ValidationResult(NamedTuple):
    name: str
    passed: bool
    actual: Any
    message: str


@dataclass(frozen=True)
class ValidationRule:
    name: str
    field: str
    minimum: float | None = None
    maximum: float | None = None
    expected: Any = None
    tolerance: float | None = None
    required: bool = True

    def check(self, metrics: Mapping[str, Any]) -> ValidationResult:
        if self.field not in metrics:
            return ValidationResult(self.name, not self.required, None,
                                    f"{'required' if self.required else 'optional'} metric missing")
        actual, f = metrics[self.field], self.field
        failures = []
        if self.minimum is not None and float(actual) < self.minimum:
            failures.append(f"{f}={actual} below {self.minimum}")
        if self.maximum is not None and float(actual) > self.maximum:
            failures.append(f"{f}={actual} above {self.maximum}")
        if self.expected is not None and self.tolerance is None and actual != self.expected:
            failures.append(f"{f}={actual} expected {self.expected}")
        if (self.expected is not None and self.tolerance is not None
                and abs(float(actual) - float(self.expected)) > self.tolerance):
            failures.append(f"{f}={actual} outside {self.expected} +/- {self.tolerance}")
        return ValidationResult(self.name, not failures, actual, "; ".join(failures) or "passed")


@dataclass(frozen=True)
class ValidationReport:
    results: list[ValidationResult]

    @property
    def passed(self) -> bool:
        return all(r.passed for r in self.results)

    @property
    def failures(self) -> list[ValidationResult]:
        return [r for r in self.results if not r.passed]


def validate_metrics(metrics: Mapping[str, Any], rules: Sequence[ValidationRule]) -> ValidationReport:
    return ValidationReport([rule.check(metrics) for rule in rules])


class ValidationRun(NamedTuple):
    name: str
    metrics: Mapping[str, Any]
    report: ValidationReport

    @property
    def passed(self) -> bool:
        return self.report.passed


class MetricStats(NamedTuple):
    count: int
    minimum: float
    maximum: float
    mean: float
    sigma: float


@dataclass(frozen=True)
class YieldSummary:
    runs: list[ValidationRun]

    @property
    def total(self) -> int:
        return len(self.runs)

    @property
    def passed(self) -> int:
        return sum(run.passed for run in self.runs)

    @property
    def failed(self) -> int:
        return self.total - self.passed

    @property
    def pass_rate(self) -> float:
        return self.passed / self.total if self.total else 0.0

    @property
    def failures(self) -> list[ValidationRun]:
        return [run for run in self.runs if not run.passed]

    def metric_stats(self, field: str) -> MetricStats:
        v = np.array([run.metrics[field] for run in self.runs if field in run.metrics], float)
        if not v.size:
            raise KeyError(f"metric '{field}' not found in any run")
        return MetricStats(v.size, float(v.min()), float(v.max()), float(v.mean()), float(v.std()))


def evaluate_metric_sets(
    metric_sets: Sequence[Mapping[str, Any]],
    rules: Sequence[ValidationRule],
    names: Sequence[str] | None = None,
) -> YieldSummary:
    names = names if names is not None else [f"run_{i}" for i in range(len(metric_sets))]
    return YieldSummary([ValidationRun(n, m, validate_metrics(m, rules))
                         for n, m in zip(names, metric_sets)])


# ── metric files (CSV/whitespace tables, .measure output, Spectre mcdata) ──

_NUMBER = r"[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?"
_NUMERIC_RE = re.compile(f"^{_NUMBER}$")
_COMMENT = ("#", "*", "//")


def _parse_float(value: str) -> float | None:
    cleaned = value.strip().strip(",;")
    return float(cleaned) if _NUMERIC_RE.match(cleaned) else None


def _data_lines(text: str) -> list[list[str]]:
    """Non-comment lines split on commas, semicolons, tabs or spaces."""
    return [[p for p in re.split(r"[\s,;]+", line.strip()) if p]
            for line in text.splitlines()
            if line.strip() and not line.lstrip().startswith(_COMMENT)]


def _parse_table(text: str) -> list[dict[str, float]]:
    """A header row of names, then rows of values; non-numeric cells are dropped."""
    lines = _data_lines(text)
    if not lines or len(lines[0]) < 2 or lines[0][0].lower().lstrip(".") == "measure":
        return []
    header = lines[0]
    if any(_parse_float(h) is not None for h in header):
        return []
    rows = []
    for parts in lines[1:]:
        if len(parts) != len(header):
            continue
        row = {k: v for k, v in ((k, _parse_float(p)) for k, p in zip(header, parts)) if v is not None}
        if row:
            rows.append(row)
    return rows


def _parse_measure_blocks(text: str) -> list[dict[str, float]]:
    """``name = value`` lines; a repeated name starts the next run."""
    rows: list[dict[str, float]] = []
    current: dict[str, float] = {}
    pattern = re.compile(rf"(?:\.?measure\s+\S+\s+)?([A-Za-z_]\w*)\s*=\s*({_NUMBER})", re.IGNORECASE)
    for line in text.splitlines():
        m = pattern.search(line)
        if not m:
            continue
        if m.group(1) in current:
            rows.append(current)
            current = {}
        current[m.group(1)] = float(m.group(2))
    return rows + [current] if current else rows


def parse_metric_rows(text: str, backend: str = "auto") -> list[dict[str, float]]:
    return _parse_table(text) or _parse_measure_blocks(text)


def _is_spectre_mcdata_file(path: Path) -> bool:
    name = path.name.lower()
    return name in {"mcdata", "processdata", "mismatchdata"} or name.endswith(".mcdata")


def _parse_spectre_mcdata(path: Path) -> list[dict[str, float]]:
    """Spectre ``scalarfile``: a bare numeric matrix, column names in the sibling
    ``paramfile`` (``mcdata`` -> ``mcparam``, ``processdata`` -> ``processParam``),
    first identifier of each line. One name short means column 0 is the iteration."""
    try:
        matrix = np.loadtxt(path, ndmin=2, comments=_COMMENT, delimiter=None)
    except ValueError:
        return []
    if not matrix.size:
        return []
    cols = matrix.shape[1]
    names: list[str] = []
    for suffix in ("param", "Param"):
        param = path.with_name(path.name[:-4] + suffix)
        if param.exists():
            names = [m.group(0) for line in param.read_text().splitlines()
                     if not line.strip().startswith(_COMMENT)
                     and (m := re.search(r"[A-Za-z_]\w*", line))]
            break
    if len(names) >= cols:
        names, iteration = names[:cols], np.arange(1, len(matrix) + 1, dtype=float)
    elif len(names) == cols - 1:
        iteration, matrix = matrix[:, 0], matrix[:, 1:]
    else:
        names, iteration = [f"value_{i + 1}" for i in range(cols)], np.arange(1, len(matrix) + 1, dtype=float)
    lead = [] if "iteration" in {n.lower() for n in names} else [("iteration", iteration)]
    columns = lead + list(zip(names, matrix.T))
    return [dict(zip([n for n, _ in columns], row)) for row in np.column_stack([c for _, c in columns]).tolist()]


def load_metric_rows(path: str | Path, backend: str = "auto") -> list[dict[str, float]]:
    path = Path(path)
    if backend.lower() == "spectre" and _is_spectre_mcdata_file(path):
        rows = _parse_spectre_mcdata(path)
        if rows:
            return rows
    return parse_metric_rows(path.read_text(), backend)


_METRIC_SUFFIXES = {".csv", ".tsv", ".txt", ".log", ".mt0", ".ms0", ".ma0"}


def _is_metric_file(path: Path, spectre: bool) -> bool:
    suffix = path.suffix.lower()
    if suffix in _METRIC_SUFFIXES:
        return True
    if not spectre:
        return False
    # Spectre scalar exports; a bare `.dat` is a waveform unless its stem says otherwise.
    return (_is_spectre_mcdata_file(path) or suffix in {".measure", ".measurement"}
            or (suffix == ".dat" and any(t in path.stem.lower()
                                         for t in ("mc", "measure", "metric", "result", "scalar"))))


def find_metric_files(path: str | Path, backend: str = "auto") -> list[Path]:
    root = Path(path)
    if root.is_file():
        return [root]
    spectre = backend.lower() == "spectre"
    return [p for p in sorted(root.rglob("*")) if p.is_file() and _is_metric_file(p, spectre)]


def load_monte_carlo_metrics(path: str | Path, backend: str = "auto") -> list[dict[str, float]]:
    return [row for f in find_metric_files(path, backend) for row in load_metric_rows(f, backend)]


def evaluate_result_text(text: str, rules: Sequence[ValidationRule], backend: str = "auto",
                         names: Sequence[str] | None = None) -> YieldSummary:
    return evaluate_metric_sets(parse_metric_rows(text, backend), rules, names)


def evaluate_result_file(path: str | Path, rules: Sequence[ValidationRule], backend: str = "auto",
                         names: Sequence[str] | None = None) -> YieldSummary:
    return evaluate_metric_sets(load_metric_rows(path, backend), rules, names)


def evaluate_monte_carlo_file(path: str | Path, rules: Sequence[ValidationRule], backend: str = "auto",
                              names: Sequence[str] | None = None) -> YieldSummary:
    return evaluate_metric_sets(load_monte_carlo_metrics(path, backend), rules, names)


# ── corners ──

@dataclass(frozen=True)
class CornerCase:
    name: str
    backend: str | None = None
    temperature: float | None = None
    nominal_temperature: float | None = None
    parameters: Mapping[str, Any] = field(default_factory=dict)
    model_libraries: Sequence[Any] = field(default_factory=tuple)
    #: Supply for this corner. Nothing applies it: the bench builder reads it,
    #: since only the builder knows which source is the supply.
    vdd: float | None = None

    def apply_to(self, bench_or_testbench: Any, backend: str = "ngspice") -> Any:
        tb = _testbench(bench_or_testbench)
        backend = self.backend or backend
        tb.with_backend(backend)
        if self.temperature is not None:
            tb.temperature = self.temperature
        if self.nominal_temperature is not None:
            tb.nominal_temperature = self.nominal_temperature
        for library in self.model_libraries:
            tb.use_pdk(library)
        card = "parameters" if backend == "spectre" else ".param"
        for key, value in self.parameters.items():
            tb.extra_line(f"{card} {key}={value}")
        return bench_or_testbench


def corner_netlists(factory: Callable[[], Any], corners: Sequence[CornerCase],
                    backend: str = "ngspice") -> dict[str, str]:
    return {c.name: _testbench(c.apply_to(factory(), backend)).netlist(c.backend or backend)
            for c in corners}


def evaluate_corners(
    factory: Callable[[], Any],
    corners: Sequence[CornerCase],
    rules: Sequence[ValidationRule],
    metric_extractor: Callable[[Any], Mapping[str, Any]],
    backend: str = "ngspice",
    runner: Callable[[Any], Any] | None = None,
) -> YieldSummary:
    runs = []
    for corner in corners:
        bench = corner.apply_to(factory(), backend)
        metrics = metric_extractor(runner(bench) if runner is not None else bench)
        runs.append(ValidationRun(corner.name, metrics, validate_metrics(metrics, rules)))
    return YieldSummary(runs)


def pdk_corners(
    library: str,
    sections: Sequence[str],
    temperatures: Sequence[float] = (27.0,),
    vdds: Sequence[float | None] = (None,),
) -> list[CornerCase]:
    """Every ``section x temperature x vdd`` of one model library, named like
    ``ss_125C_1.62V``. ``sections`` are the library's own ``.lib`` names."""
    from spicerack import ModelLibrary

    return [CornerCase(f"{s}_{t:g}C" + ("" if v is None else f"_{v:g}V"), temperature=t, vdd=v,
                       model_libraries=(ModelLibrary(library, corner=s),))
            for s in sections for t in temperatures for v in vdds]


def run_corners(
    build: Callable[[CornerCase], Any],
    measure: Callable[[Any], Mapping[str, float]],
    corners: Sequence[CornerCase] = (),
    *,
    seeds: int | Sequence[int] | None = None,
    backend: str = "ngspice",
) -> list[dict[str, Any]]:
    """Run one bench over every corner (and Monte Carlo seed); one row per run.

    ``build(corner)`` returns a fresh ``Testbench`` (or ``DesignBench``) and
    reads ``corner.vdd`` itself; the runner then applies the corner's
    temperature, model libraries and ``.param`` values, plus ``.options
    seed=<n>`` for each seed. ``measure(tb)`` runs the analysis and returns
    ``{metric: value}``. No corners means one ``nominal`` corner.

    ``seeds=N`` means seeds ``1..N``. A seed only changes anything when the
    models are statistical (e.g. a PDK ``*_mm`` / mismatch section, or
    ``agauss`` parameters); ngspice draws are reproducible per seed.

    Each row is ``{"corner", "seed", "temperature", "vdd", **metrics}``. A run
    that raises gets ``"error": str`` instead of metrics, so one
    non-converging corner does not discard the rest of the sweep.
    """
    if isinstance(seeds, int):
        seeds = range(1, seeds + 1)
    rows: list[dict[str, Any]] = []
    for corner in corners or (CornerCase("nominal"),):
        for seed in seeds if seeds is not None else (None,):
            row: dict[str, Any] = {"corner": corner.name, "seed": seed,
                                   "temperature": corner.temperature, "vdd": corner.vdd}
            try:
                bench = corner.apply_to(build(corner), backend)
                if seed is not None:
                    _testbench(bench).options(seed=seed)
                row.update(measure(bench))
            except Exception as exc:  # noqa: BLE001 - recorded in the row
                row["error"] = f"{type(exc).__name__}: {exc}"
            rows.append(row)
    return rows


# ── Spectre Monte Carlo statement ──

@dataclass(frozen=True)
class MonteCarloPlan:
    """Statistical sampling plan. Spectre is the only backend with a Monte
    Carlo statement SpiceRack can emit. ``distributions`` is not read."""

    backend: str = "spectre"
    samples: int = 100
    distributions: Mapping[str, str] = field(default_factory=dict)
    spectre_inner: str = "tran1"
    spectre_inner_type: str = "tran"
    seed: int | None = None

    def apply_to(self, bench_or_testbench: Any) -> Any:
        if self.backend.lower() != "spectre":
            raise ValueError(f"MonteCarloPlan supports backend='spectre'; got {self.backend!r}")
        tb = _testbench(bench_or_testbench)
        tb.with_backend(self.backend)
        tb.add_spectre_monte_carlo(self.samples, self.spectre_inner, self.spectre_inner_type, self.seed)
        return bench_or_testbench


def monte_carlo_netlist(bench_or_testbench: Any, plan: MonteCarloPlan) -> str:
    return _testbench(plan.apply_to(bench_or_testbench)).netlist(plan.backend)
