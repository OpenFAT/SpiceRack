"""Data-converter testbenches: ADC dynamic and static linearity, comparator
threshold and delay. DAC and sample-and-hold benches are in ``converters_2``.

Every bench drives its own stimulus, and ``bench.measure()`` runs its own
analysis. Spectra are coherent (``coherent_bin``) and use a rectangular window,
so every bin other than DC, the tone and its harmonics is noise.
"""

from __future__ import annotations

import math
from typing import Any, Sequence

import numpy as np

from ._digital import _codes
from .analysis import crossings
from .design import DesignBench, _step_pwl

#: ENOB = (SNDR - 10 log10(1.5)) / (20 log10 2): the 1.76 and 6.02 unrounded.
_ENOB_OFFSET_DB = 10 * math.log10(1.5)
_DB_PER_BIT = 20 * math.log10(2)


def _is_prime(n: int) -> bool:
    return n > 1 and all(n % d for d in range(2, math.isqrt(n) + 1))


def _fold(k: int, n: int) -> int:
    """Bin of harmonic index ``k`` (mod ``n``) aliased into the first Nyquist zone."""
    b = k % n
    return min(b, n - b)


def coherent_bin(record: int, target_fraction: float, harmonics: int = 5) -> int:
    """The signal bin J for a coherent ``record``-point sine: fin = J * fs / record.

    J is the odd prime nearest ``target_fraction * record`` whose folded
    harmonics 2..``harmonics`` land on distinct bins, none on DC or on J. A
    prime J is coprime to a power-of-two record, so every sample lands on a
    distinct phase of the sine.
    """
    if record < 8 or record & (record - 1):
        raise ValueError(f"record must be a power of two >= 8, got {record}")
    if not 0 < target_fraction < 0.5:
        raise ValueError(f"target_fraction must lie in (0, 0.5), got {target_fraction}")
    for j in sorted(range(3, record // 2), key=lambda j: abs(j - target_fraction * record)):
        bins = [_fold(k * j, record) for k in range(2, harmonics + 1)]
        if _is_prime(j) and len(set(bins)) == len(bins) and 0 not in bins and j not in bins:
            return j
    raise ValueError(f"no prime bin near {target_fraction} of {record} keeps {harmonics} harmonics apart")


def _tone(x: np.ndarray, signal_bin: int, harmonics: int, band: int | None = None,
          alias: int | None = None) -> dict[str, float]:
    """Single-tone figures of a coherent record ``x``, rectangular window.

    Bins 1..``band`` (default: Nyquist) form the band. Harmonics 2..``harmonics``
    fold modulo ``alias`` bins (default ``len(x)``; the update count for a record
    oversampled between updates) and count only inside the band. SFDR counts
    any spur, not only harmonics. ``amplitude`` is the tone's peak, in x's unit.
    """
    n = len(x)
    spectrum = np.fft.rfft(x)
    power = np.abs(spectrum) ** 2
    power[1:(n + 1) // 2] *= 2  # one-sided; an even record's Nyquist bin is not mirrored
    band = n // 2 if band is None else band
    assert 0 < signal_bin <= band <= n // 2, (signal_bin, band, n)
    harmonic = {k: _fold(k * signal_bin, alias or n) for k in range(2, harmonics + 1)}
    harmonic = {k: b for k, b in harmonic.items() if 0 < b <= band and b != signal_bin}
    inband = power[1:band + 1]
    p_sig = power[signal_bin]
    p_harm = sum(power[b] for b in set(harmonic.values()))
    p_noise = inband.sum() - p_sig - p_harm
    spurs = np.delete(inband, signal_bin - 1)
    db = lambda ratio: 10 * math.log10(ratio) if ratio > 0 else -math.inf
    sndr = db(p_sig / (p_noise + p_harm))
    return {
        "sndr_db": sndr,
        "snr_db": db(p_sig / p_noise),
        "thd_db": db(p_harm / p_sig),
        "sfdr_dbc": db(p_sig / spurs.max()),
        "hd2_db": db(power[harmonic[2]] / p_sig) if 2 in harmonic else math.nan,
        "hd3_db": db(power[harmonic[3]] / p_sig) if 3 in harmonic else math.nan,
        "enob_bits": (sndr - _ENOB_OFFSET_DB) / _DB_PER_BIT,
        "amplitude": 2 * abs(spectrum[signal_bin]) / n,
    }


def _clock(tb: Any, node: str, reference_node: str, period: float, high: float,
           reset_first: bool = False) -> None:
    """50 % clock, edges period/1000. High over [kT, kT + T/2) by default;
    ``reset_first`` puts the low half first, so the high half evaluates."""
    lo, hi = (high, 0.0) if reset_first else (0.0, high)
    tb.PulseVoltageSource(name="clk", positive=node, negative=reference_node, initial_value=lo,
                          pulsed_value=hi, pulse_width=period / 2, period=period,
                          rise_time=period / 1000, fall_time=period / 1000)


def _transient(tb: Any, step: float, end: float, max_step: float) -> Any:
    """Queue the transient on ``tb`` for the netlist, and return the bench's runner."""
    tb.add_transient(step_time=step, end_time=end, max_time=max_step)
    return lambda: tb.transient(step_time=step, end_time=end, max_time=max_step)


def adc_dynamic(
    ps: Any,
    dut: Any,
    *,
    output_nodes: list[str],
    input_node: str = "vin",
    input_node_n: str | None = None,
    clock_node: str = "clk",
    reference_node: str = "0",
    full_scale: tuple[float, float] = (0.0, 1.0),
    conversion_period: float = 1e-6,
    record: int = 1024,
    target_fraction: float = 0.1,
    amplitude_dbfs: float = -0.5,
    settle_conversions: int = 16,
    logic_high: float = 1.0,
    sample_fraction: float = 0.9,
    harmonics: int = 5,
    reltol: float | None = None,
) -> DesignBench:
    """SNDR, SNR, SFDR, THD and ENOB of a Nyquist ADC from a coherent sine.

    The clock is high (track) over the first half of each period and samples
    on its falling edge; the code is read at ``(k + sample_fraction) * T``, LSB
    first from ``output_nodes``. The first ``settle_conversions`` codes are
    discarded (cover the DUT's pipeline latency here); the next ``record``, a
    power of two, form the spectrum. ``input_node_n`` drives the input
    differentially about mid-scale, half the swing on each side.

    ``signal_dbfs`` is the measured tone against the 2^N-code full scale;
    ``enob_fs_bits`` refers SNDR to full scale (IEEE 1241). ``outlier_count``
    counts codes more than 4 LSB from a 3-parameter sine fit: sparkle codes
    and metastable decisions that hide in an FFT floor. ``reltol`` defaults
    to 1e-4 up to 10 bits and 1e-5 above: the default 1e-3 moved a -55 dBc
    HD3 by 0.17 dB on an ideal 16-bit quantizer.
    """
    lo, hi = full_scale
    n_bits = len(output_nodes)
    j = coherent_bin(record, target_fraction, harmonics)
    fin = j / (record * conversion_period)
    amplitude = (hi - lo) / 2 * 10 ** (amplitude_dbfs / 20)
    mid = (hi + lo) / 2
    tb = ps.Testbench(dut)
    if input_node_n is None:
        tb.SinusoidalVoltageSource(name="in", positive=input_node, negative=reference_node,
                                   offset=mid, amplitude=amplitude, frequency=fin)
    else:
        for name, node, sign in (("inp", input_node, 1), ("inn", input_node_n, -1)):
            tb.SinusoidalVoltageSource(name=name, positive=node, negative=reference_node,
                                       offset=mid, amplitude=sign * amplitude / 2, frequency=fin)
    _clock(tb, clock_node, reference_node, conversion_period, logic_high)
    tb.options(reltol=reltol or (1e-4 if n_bits <= 10 else 1e-5))
    tb.save(*(f"V({node})" for node in output_nodes))
    total = settle_conversions + record
    runner = _transient(tb, conversion_period / 50, total * conversion_period,
                        min(conversion_period / 20, 1 / (100 * fin)))

    def extract(tran: Any) -> dict[str, float]:
        k = settle_conversions + np.arange(record)
        codes = _codes(tran, output_nodes, (k + sample_fraction) * conversion_period, logic_high).astype(float)
        m = _tone(codes, j, harmonics)
        signal_dbfs = 20 * math.log10(m.pop("amplitude") / 2 ** (n_bits - 1))
        phase = 2 * np.pi * j * np.arange(record) / record
        basis = np.column_stack([np.sin(phase), np.cos(phase), np.ones(record)])
        fit = basis @ np.linalg.lstsq(basis, codes, rcond=None)[0]
        return m | {
            "enob_fs_bits": (m["sndr_db"] - _ENOB_OFFSET_DB - signal_dbfs) / _DB_PER_BIT,
            "signal_dbfs": signal_dbfs,
            "input_frequency_hz": fin,
            "outlier_count": float(np.count_nonzero(np.abs(codes - fit) > 4.0)),
        }

    return DesignBench(
        name="adc_dynamic",
        category="adc",
        testbench=tb,
        intent="Single-tone SNDR, SNR, SFDR, THD and ENOB from a coherent sine.",
        # Conversion time of an asynchronous SAR needs its done/valid node; not wired.
        measurements=["sndr_db", "snr_db", "thd_db", "sfdr_dbc", "hd2_db", "hd3_db", "enob_bits",
                      "enob_fs_bits", "signal_dbfs", "input_frequency_hz", "outlier_count"],
        result_fields=["tran.time", "digital output nodes"],
        validation=["SNDR above target at stated amplitude and fin", "no outlier codes"],
        extractor=extract,
        runner=runner,
    )


def _transitions_ramp(codes: np.ndarray, levels: int, v_first: float, dv: float) -> np.ndarray:
    """T[k], k = 1..levels-1: midpoint between the last sample below code k and
    the first at or above it, for samples v_first + i*dv."""
    below = np.cumsum(np.bincount(codes, minlength=levels))[:-1]
    return v_first + dv * (below - 0.5)


def _transitions_sine(codes: np.ndarray, levels: int, centre: float, amplitude: float) -> np.ndarray:
    """T[k] from the arcsine density of a coherent sine (IEEE 1241 code-density test)."""
    below = np.cumsum(np.bincount(codes, minlength=levels))[:-1]
    return centre - amplitude * np.cos(np.pi * below / len(codes))


def adc_static_linearity(
    ps: Any,
    dut: Any,
    *,
    output_nodes: list[str],
    input_node: str = "vin",
    clock_node: str = "clk",
    reference_node: str = "0",
    full_scale: tuple[float, float] = (0.0, 1.0),
    conversion_period: float = 1e-6,
    stimulus: str = "ramp",
    samples_per_code: int | None = None,
    overdrive: float = 0.02,
    first_transition_lsb: float = 1.0,
    latency_conversions: int = 0,
    logic_high: float = 1.0,
    sample_fraction: float = 0.9,
) -> DesignBench:
    """INL, DNL, missing codes, offset and gain by code density (histogram).

    ``stimulus="ramp"`` sweeps ``overdrive`` past both ends of ``full_scale``
    over ``samples_per_code`` (default 16) conversions per code; transition k
    is the midpoint between the last sample below code k and the first at or
    above, so each transition is exact to half a ramp step. ``"sine"`` (IEEE
    1241) drives a coherent sine overdriving both ends, ``samples_per_code``
    (default 256) times 2^N samples rounded up to a power of two, and places
    the transitions from the arcsine density. End codes are excluded.

    DNL and INL use the end-point fit; ``offset_error_lsb`` is T[1] against
    ``lo + first_transition_lsb`` LSB (1.0 truncating, 0.5 rounding). The code
    read at conversion ``k`` belongs to the sample of ``k - latency_conversions``.
    Monotonicity is not observable from a histogram; ``adc_ramp`` reports it.
    """
    if stimulus not in ("ramp", "sine"):
        raise ValueError(f"stimulus is 'ramp' or 'sine', got {stimulus!r}")
    lo, hi = full_scale
    levels = 2 ** len(output_nodes)
    lsb = (hi - lo) / levels
    period = conversion_period
    tb = ps.Testbench(dut)
    if stimulus == "ramp":
        spc = samples_per_code or 16
        samples = math.ceil(spc * levels * (1 + 2 * overdrive))
        v0 = lo - overdrive * (hi - lo)
        dv = (hi - lo) * (1 + 2 * overdrive) / samples
        # Sample i is taken on the falling clock edge at (i + 1/2) T.
        total = samples + latency_conversions
        tb.PieceWiseLinearVoltageSource(name="ramp", positive=input_node, negative=reference_node,
                                        values=[(0.0, v0 - dv / 2), (total * period, v0 + (total - 0.5) * dv)])
        transitions = lambda codes: _transitions_ramp(codes, levels, v0, dv)
        max_step = period / 20
    else:
        spc = samples_per_code or 256
        samples = 1 << math.ceil(math.log2(spc * levels))
        j = coherent_bin(samples, 0.1)
        fin = j / (samples * period)
        centre, amplitude = (hi + lo) / 2, (hi - lo) / 2 * (1 + overdrive)
        tb.SinusoidalVoltageSource(name="in", positive=input_node, negative=reference_node,
                                   offset=centre, amplitude=amplitude, frequency=fin)
        total = samples + latency_conversions
        transitions = lambda codes: _transitions_sine(codes, levels, centre, amplitude)
        max_step = min(period / 20, 1 / (100 * fin))
    _clock(tb, clock_node, reference_node, period, logic_high)
    tb.save(*(f"V({node})" for node in output_nodes))
    runner = _transient(tb, period / 50, total * period, max_step)

    def extract(tran: Any) -> dict[str, float]:
        k = latency_conversions + np.arange(samples)
        codes = _codes(tran, output_nodes, (k + sample_fraction) * period, logic_high)
        t = transitions(codes)  # t[k-1] is transition k
        lsb_ep = (t[-1] - t[0]) / (levels - 2)
        dnl = np.diff(t) / lsb_ep - 1
        inl = (t - (t[0] + np.arange(levels - 1) * lsb_ep)) / lsb_ep
        counts = np.bincount(codes, minlength=levels)
        return {
            "dnl_max_lsb": float(dnl.max()),
            "dnl_min_lsb": float(dnl.min()),
            "inl_max_lsb": float(np.abs(inl).max()),
            "missing_codes": float(np.count_nonzero(counts[1:-1] == 0)),
            "offset_error_lsb": float((t[0] - (lo + first_transition_lsb * lsb)) / lsb),
            "gain_error": float(lsb_ep / lsb - 1),
        }

    return DesignBench(
        name="adc_static_linearity",
        category="adc",
        testbench=tb,
        intent="Static transfer by code density: DNL, INL, missing codes, offset and gain.",
        measurements=["dnl_max_lsb", "dnl_min_lsb", "inl_max_lsb", "missing_codes",
                      "offset_error_lsb", "gain_error"],
        result_fields=["tran.time", "digital output nodes"],
        validation=["no missing codes", "DNL and INL in budget", "offset and gain in budget"],
        extractor=extract,
        runner=runner,
    )


def _differential(tb: Any, inp: str, inn: str, reference_node: str, vcm: float,
                  vd: np.ndarray, period: float) -> None:
    """inp = vcm + vd/2 and inn = vcm - vd/2, level k over [kT, (k+1)T)."""
    _step_pwl(tb, "inp", inp, reference_node, vcm + vd / 2, period)
    _step_pwl(tb, "inn", inn, reference_node, vcm - vd / 2, period)


def _decisions(tran: Any, outp: str, outn: str | None, times: Any, logic_high: float) -> np.ndarray:
    """1 where the comparator decided "inp > inn": outp - outn > 0, or outp above VH/2."""
    t = np.asarray(tran.time, float)
    v = np.interp(times, t, np.asarray(tran[outp], float))
    if outn is None:
        return (v > logic_high / 2).astype(int)
    return (v - np.interp(times, t, np.asarray(tran[outn], float)) > 0).astype(int)


def comparator_threshold(
    ps: Any,
    dut: Any,
    *,
    inp: str = "inp",
    inn: str = "inn",
    clock_node: str | None = "clk",
    out_node: str = "outp",
    outn_node: str | None = None,
    reference_node: str = "0",
    vcm: float = 0.5,
    span: float = 20e-3,
    steps: int = 400,
    clock_period: float = 10e-9,
    logic_high: float = 1.0,
    sample_fraction: float = 0.9,
) -> DesignBench:
    """Input-referred offset and hysteresis from one up-then-down staircase.

    The differential input steps -span -> +span -> -span in ``2*span/steps``
    increments, one per clock, at constant ``vcm``. The clock is low (reset)
    over the first half of each period and high (evaluate) over the second;
    the input changes at the start of reset and the decision is read at
    ``(k + 1/2 + sample_fraction/2) T``. ``clock_node=None`` drives no clock,
    for a continuous comparator.

    Each threshold is the midpoint of the two levels around the last flip, so
    it is exact to ``resolution_v / 2``. ``hysteresis_v`` in a clocked
    comparator is memory: incomplete reset. Ideal sources hide kickback; offset
    against common mode is one bench per ``vcm``.
    """
    step = 2 * span / steps
    vd = np.concatenate([-span + step * np.arange(steps + 1), span - step * np.arange(1, steps + 1)])
    tb = ps.Testbench(dut)
    _differential(tb, inp, inn, reference_node, vcm, vd, clock_period)
    if clock_node is not None:
        _clock(tb, clock_node, reference_node, clock_period, logic_high, reset_first=True)
    tb.save(*(f"V({n})" for n in (inp, inn, out_node, outn_node) if n is not None))
    runner = _transient(tb, clock_period / 100, len(vd) * clock_period, clock_period / 50)

    def last_flip(d: np.ndarray, v: np.ndarray, before: int) -> tuple[float, int]:
        idx = np.nonzero(d == before)[0]
        if len(idx) == 0 or idx[-1] == len(d) - 1:
            raise ValueError(f"comparator never left {before} on a half-ramp of +-{span} V; widen span")
        return (v[idx[-1]] + v[idx[-1] + 1]) / 2, int(np.count_nonzero(np.diff(d)))

    def extract(tran: Any) -> dict[str, float]:
        times = (np.arange(len(vd)) + 0.5 + sample_fraction / 2) * clock_period
        d = _decisions(tran, out_node, outn_node, times, logic_high)
        up, flips_up = last_flip(d[:steps + 1], vd[:steps + 1], 0)
        down, flips_down = last_flip(d[steps:], vd[steps:], 1)
        return {
            "threshold_up_v": float(up),
            "threshold_down_v": float(down),
            "offset_v": float((up + down) / 2),
            "hysteresis_v": float(up - down),
            "chatter_count": float(flips_up + flips_down - 2),
            "resolution_v": step,
        }

    return DesignBench(
        name="comparator_threshold",
        category="comparator",
        testbench=tb,
        intent="Input-referred offset, hysteresis (memory) and decision chatter.",
        measurements=["threshold_up_v", "threshold_down_v", "offset_v", "hysteresis_v",
                      "chatter_count", "resolution_v"],
        result_fields=["tran.time", f"tran['{out_node}']"],
        validation=["offset within budget", "hysteresis below budget", "no chatter"],
        extractor=extract,
        runner=runner,
    )


def comparator_delay(
    ps: Any,
    dut: Any,
    *,
    inp: str = "inp",
    inn: str = "inn",
    clock_node: str = "clk",
    outp_node: str = "outp",
    outn_node: str | None = None,
    reference_node: str = "0",
    vcm: float = 0.5,
    logic_high: float = 1.0,
    clock_period: float = 1e-9,
    overdrives: Sequence[float] = (1e-1, 1e-2, 1e-3, 1e-4, 1e-5, 1e-6),
    decide_level: float = 0.2,
    fit_below: float = 1e-3,
    max_step: float | None = None,
) -> DesignBench:
    """Decision delay against overdrive, regeneration time constant and
    metastability window from one sign-alternating run.

    Cycle 2i applies +overdrives[i] and cycle 2i+1 applies -overdrives[i], so
    each decision unwinds the last. Clock as in ``comparator_threshold``
    (reset low, evaluate high). The delay runs from the rising clock crossing
    VH/2 to ``|outp - outn|`` reaching ``decide_level`` (single-ended: to the
    output crossing VH/2); 0 when it is already there, inf when never.

    ``tau_regen_s`` is -d t_d / d ln(Vod) fitted over overdrives up to
    ``fit_below``; ``v_meta_v`` is the overdrive still deciding within the
    half period, Vod_min * exp(-(T/2 - t_d(Vod_min)) / tau). The solver runs
    at reltol 1e-6, vntol 1e-9 for microvolt inputs. Set ``max_step`` to
    tau/200 or less (default clock_period/1000): on a tanh latch tau/50 put
    the 1 uV delay 0.6 % of tau off, tau/200 0.003 %.
    """
    vd = np.ravel([[od, -od] for od in overdrives])
    tb = ps.Testbench(dut)
    _differential(tb, inp, inn, reference_node, vcm, vd, clock_period)
    _clock(tb, clock_node, reference_node, clock_period, logic_high, reset_first=True)
    tb.options(reltol=1e-6, vntol=1e-9, abstol=1e-15)
    tb.save(*(f"V({n})" for n in (clock_node, outp_node, outn_node) if n is not None))
    runner = _transient(tb, clock_period / 1000, len(vd) * clock_period, max_step or clock_period / 1000)

    def extract(tran: Any) -> dict[str, float]:
        t = np.asarray(tran.time, float)
        out = np.asarray(tran[outp_node], float)
        if outn_node is not None:
            out = np.abs(out - np.asarray(tran[outn_node], float))
        level = decide_level if outn_node is not None else logic_high / 2
        rises = crossings(t, np.asarray(tran[clock_node], float), logic_high / 2, rising=True)
        edges = [next(e for e in rises if k * clock_period <= e < (k + 1) * clock_period) for k in range(len(vd))]
        hits = np.asarray(crossings(t, out, level, rising=True if outn_node is not None else None))
        delay = np.empty(len(vd))
        for k, e in enumerate(edges):
            if outn_node is not None and np.interp(e, t, out) >= level:
                delay[k] = 0.0
                continue
            after = hits[(hits >= e) & (hits < (k + 1) * clock_period)]
            delay[k] = after[0] - e if len(after) else math.inf
        worst = delay.reshape(-1, 2).max(axis=1)  # per overdrive, worse sign
        od = np.asarray(overdrives, float)
        fit = np.repeat(od, 2) <= fit_below
        if len(np.unique(np.repeat(od, 2)[fit])) < 2:
            raise ValueError(f"tau needs two overdrives at or below fit_below={fit_below}")
        tau = -np.polyfit(np.log(np.repeat(od, 2)[fit]), delay[fit], 1)[0]
        ref = int(np.argmin(od))
        decided = _decisions(tran, outp_node, outn_node, (np.arange(len(vd)) + 0.95) * clock_period, logic_high)
        return {
            "delay_max_od_s": float(worst[int(np.argmax(od))]),
            "delay_min_od_s": float(worst[ref]),
            "tau_regen_s": float(tau),
            "v_meta_v": float(od[ref] * math.exp(-(clock_period / 2 - worst[ref]) / tau)),
            "decision_errors": float(np.count_nonzero(decided != (vd > 0))),
            "delay_sign_skew_s": float(np.abs(np.diff(delay.reshape(-1, 2), axis=1)).max()),
        }

    return DesignBench(
        name="comparator_delay",
        category="comparator",
        testbench=tb,
        intent="Decision delay against overdrive, regeneration tau and metastability window.",
        measurements=["delay_max_od_s", "delay_min_od_s", "tau_regen_s", "v_meta_v",
                      "decision_errors", "delay_sign_skew_s"],
        result_fields=["tran.time", f"tran['{outp_node}']", f"tran['{clock_node}']"],
        validation=["delay at minimum overdrive within the half period", "no decision errors"],
        extractor=extract,
        runner=runner,
    )
