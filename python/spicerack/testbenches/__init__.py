"""Reusable analog and mixed-signal testbench builders."""

from .analysis import (
    CornerCase,
    MetricSpec,
    MonteCarloPlan,
    ValidationRule,
    corner_netlists,
    evaluate_corners,
    evaluate_metric_sets,
    evaluate_monte_carlo_file,
    evaluate_result_file,
    evaluate_result_text,
    extract_metrics,
    find_metric_files,
    load_metric_rows,
    load_monte_carlo_metrics,
    monte_carlo_netlist,
    parse_metric_rows,
    pdk_corners,
    run_corners,
    validate_metrics,
)
from .amplifiers import loop_gain, noise_spectrum, opamp_open_loop, rf_compression_ip3, step_response
from .amplifiers_2 import cmrr, harmonic_distortion, offset_icmr, output_swing, port_impedance, rf_two_port
from .bias import (
    charge_pump_current,
    current_mirror_output,
    current_reference,
    switched_cap_transfer,
    transconductor_gm,
)
from .characterize import bisect_boundary, delay_table, hold_time, input_capacitance, setup_time
from .design import (
    DesignBench,
    adc_ramp,
    amplifier_current_gain,
    amplifier_transimpedance,
    amplifier_voltage_gain,
    bandgap_reference,
    bandgap_tempco,
    charge_amplifier,
    dac_static_linearity,
    demux_routing,
    mux_routing,
    pll_lock,
    sample_hold,
    switch_characterization,
)
from .converters import (
    adc_dynamic,
    adc_static_linearity,
    coherent_bin,
    comparator_delay,
    comparator_threshold,
)
from .converters_2 import (
    dac_dynamic,
    dac_transition,
    sample_hold_dynamic,
    sample_hold_feedthrough,
    sample_hold_track_bandwidth,
)
from .converters_3 import comparator_kickback, dac_output_impedance, pipeline_stage_residue
from .timing import (
    clock_buffer_timing,
    crystal_negative_resistance,
    delay_line_range,
    oscillator_frequency,
    oscillator_jitter,
    oscillator_sweep,
)
from .timing_2 import dll_lock, pfd_cp_transfer, pll_lock_time, pll_loop_margins, pll_reference_spur
from .power import (
    average_power,
    regulator_line_sweep,
    regulator_load_sweep,
    regulator_loop_margins,
    regulator_step,
    supply_rejection,
    supply_startup,
)
