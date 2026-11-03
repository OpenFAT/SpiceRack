//! Each analysis -> its documented Spectre statement (dc, ac, tran, noise,
//! xf, sens, pss/hb/sp/stb, pac/pnoise/pxf/pstb, sweep, montecarlo, fourier),
//! SI scale factors, and the analyses Spectre refuses.
//!
//! Cites: `[REF]`, `[CMP]` and `[KUN]` (see `main.rs`), per test.
//!
//! Contents: none (tests only).

use super::*;

// ---- private ----

/// `[REF]` p.20143 verbatim `dc1 dc` (a `dc` without a sweep parameter is the
/// operating point).
/// `[REF]` p.64: temperature is `param=temp`, a netlist parameter is a bare
/// `param=<name>`, and a device parameter needs `dev=<instance>` alongside
/// `param=<parameter>`. SPICE `.dc Vin ...` sweeps the source's `dc` value
/// (`[CMP]` p.683 vsource parameter #1).
#[test]
fn spectre_dc_sweeps_pick_the_documented_selector() {
    assert_eq!(analysis(Analysis::Op), "op1 dc");

    let dc = |src: &str| {
        analysis(Analysis::Dc {
            sweeps: vec![DcSweep {
                source: src.into(),
                start: 0.0,
                stop: 5.0,
                step: 0.1,
            }],
        })
    };
    assert_eq!(
        dc("Vin"),
        "dc1 dc dev=vin param=dc start=0 stop=5 step=100m"
    );
    assert_eq!(
        dc("Iin"),
        "dc1 dc dev=iin param=dc start=0 stop=5 step=100m"
    );
    assert_eq!(dc("temp"), "dc1 dc param=temp start=0 stop=5 step=100m");
    assert_eq!(dc("rload"), "dc1 dc param=rload start=0 stop=5 step=100m");
}

/// `[REF]` p.43-44 (ac) and p.416-417 (tran): `Name ac ...` /
/// `Name tran ...`. tran parameters `stop` (#1), `start` (#3),
/// `maxstep` (#8), `step` (#9). Verbatim `[REF]` p.433
/// `tran1 tran stop=0.5u noisefmax=10G noiseseed=1`.
#[test]
fn spectre_ac_and_tran_match_documented_parameters() {
    assert_eq!(
        analysis(Analysis::Ac {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 100,
                start: 1.0,
                stop: 1e9
            }
        }),
        "ac1 ac start=1 stop=1G dec=100",
    );
    assert_eq!(
        analysis(Analysis::Transient {
            step: 1e-9,
            stop: 1e-6,
            start: Some(1e-7),
            max_step: Some(5e-9),
            uic: false,
        }),
        "tran1 tran step=1n stop=1u start=100n maxstep=5n",
    );
}

/// `[REF]` "Sweep interval parameters" (p.44, p.182, p.389, p.394, p.439, ...)
/// list `step lin dec log values valuesfile` only — Spectre has no octave
/// sweep, so SPICE's `oct` must be refused rather than emitted.
#[test]
fn spectre_rejects_octave_frequency_sweeps() {
    let r = try_analysis(&Analysis::Ac {
        sweep: FreqSweep {
            variation: Sweep::Oct,
            points: 10,
            start: 1.0,
            stop: 1e6,
        },
    });
    assert!(
        matches!(r, Err(CodeGenError::UnsupportedAnalysis { .. })),
        "{r:?}"
    );
}

/// `[REF]` p.181-182 noise: `Name [p] [n] noise parameter=value ...`,
/// "The optional terminals (p and n) specify the output of the circuit",
/// `oprobe` (#16) and `iprobe` (#17) name *components*. The old codegen passed
/// `oprobe=V(out)`, which is neither a component nor Spectre node syntax.
#[test]
fn spectre_noise_puts_the_output_in_the_terminal_list() {
    assert_eq!(
        analysis(Analysis::Noise {
            output: "out".into(),
            reference: "0".into(),
            source: "Vin".into(),
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1.0,
                stop: 1e6
            },
            points_per_summary: None,
        }),
        "noise1 (out 0) noise start=1 stop=1M dec=10 iprobe=vin",
    );
}

/// `[REF]` p.438-439 xf: "you can simply specify a voltage to be the output by
/// giving a pair of nodes on the xf analysis statement". xf computes the
/// transfer function from *every* independent source, so there is no
/// `source=` parameter — the old codegen emitted one.
///
/// INFERRED, NOT VERIFIED: `freq=0` as the stand-in for SPICE's DC `.tf`.
#[test]
fn spectre_xf_has_no_source_parameter() {
    let s = analysis(Analysis::Tf {
        output: "V(out)".into(),
        source: "Vin".into(),
    });
    assert_eq!(s, "xf1 (out 0) xf freq=0");
    assert!(!s.contains("source="), "{s}");
}

/// `[REF]` p.524 verbatim: `sens (1 n2 7) for (analAC)` — `sens` is a control
/// statement naming an analysis, spelled
/// `sens (outputs) to (params) for (analyses)`.
#[test]
fn spectre_sens_matches_documented_control_statement() {
    assert_eq!(
        analysis(Analysis::Sensitivity {
            output: "V(out)".into(),
            ac: None
        }),
        "sensdc1 dc\nsens (out) for (sensdc1)",
    );
    assert_eq!(
        analysis(Analysis::Sensitivity {
            output: "out".into(),
            ac: Some(FreqSweep {
                variation: Sweep::Dec,
                points: 10,
                start: 1.0,
                stop: 1e6
            }),
        }),
        "sensac1 ac start=1 stop=1M dec=10\nsens (out) for (sensac1)",
    );
}

/// `[REF]` p.272-274 pss: `fund` (#2), `harms` (#4), `tstab` (#6), and
/// optional `[p] [n]` output terminals. There is no `ppv` and no `probe`.
/// `[REF]` p.90-91 hb: `fundfreqs=[...]` (#2) and `maxharms=[...]` (#3), not
/// `toneN`/`nharmN`.
/// `[REF]` p.387 sp / p.393 stb.
#[test]
fn spectre_rf_large_signal_analyses_match_documented_parameters() {
    assert_eq!(
        analysis(Analysis::Pss {
            fundamental: 1e9,
            stabilization: 1e-7,
            observe_node: "out".into(),
            points_per_period: 128,
            harmonics: 10,
            sc_iter: None,
            steady_coeff: None,
            uic: false,
        }),
        "pss1 (out 0) pss fund=1G tstab=100n harms=10",
    );
    assert_eq!(
        analysis(Analysis::HarmonicBalance {
            frequencies: vec![1e9, 1.01e9],
            harmonics: vec![5, 3],
        }),
        "hb1 hb fundfreqs=[1G 1.01G] maxharms=[5 3]",
    );
    assert_eq!(
        analysis(Analysis::SPar {
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 20,
                start: 1e6,
                stop: 1e9
            }
        }),
        "sp1 sp start=1M stop=1G dec=20",
    );
    assert_eq!(
        analysis(Analysis::Stability {
            probe: "IPRB0".into(),
            sweep: FreqSweep {
                variation: Sweep::Dec,
                points: 20,
                start: 1.0,
                stop: 1e9
            },
        }),
        "stb1 stb start=1 stop=1G dec=20 probe=iprb0",
    );
}

/// `[REF]` p.245-246 (pac, `sweeptype`), p.253-256 (pnoise — probe parameters
/// are `oprobe`/`iprobe`, there is no `refprobe`), p.303-304 (pxf — the only
/// probe parameter is `probe`, there is no `isrc`), p.298-299 (pstb, `probe`).
#[test]
fn spectre_periodic_small_signal_analyses_drop_invented_parameters() {
    let pac = analysis(Analysis::SpectrePac {
        pss: PssBase {
            pss_fundamental: 1e9,
            pss_stabilization: 1e-7,
            pss_harmonics: 10,
        },
        sweep: FreqSweep {
            variation: Sweep::Dec,
            points: 100,
            start: 1.0,
            stop: 1e9,
        },
        sweep_type: "relative".into(),
    });
    assert_eq!(
        pac,
        "pss1 pss fund=1G tstab=100n harms=10\n\
         pac1 pac start=1 stop=1G dec=100 sweeptype=relative",
    );

    let pnoise = analysis(Analysis::SpectrePnoise {
        pss: PssBase {
            pss_fundamental: 1e9,
            pss_stabilization: 1e-7,
            pss_harmonics: 10,
        },
        output: "out".into(),
        reference: "0".into(),
        sweep: FreqSweep {
            variation: Sweep::Dec,
            points: 10,
            start: 1.0,
            stop: 1e6,
        },
    });
    assert_eq!(
        pnoise,
        "pss1 pss fund=1G tstab=100n harms=10\n\
         pnoise1 (out 0) pnoise start=1 stop=1M dec=10",
    );
    assert!(!pnoise.contains("refprobe"), "{pnoise}");

    let pxf = analysis(Analysis::SpectrePxf {
        pss: PssBase {
            pss_fundamental: 1e9,
            pss_stabilization: 1e-7,
            pss_harmonics: 10,
        },
        output: "out".into(),
        source: "Vin".into(),
        sweep: FreqSweep {
            variation: Sweep::Dec,
            points: 10,
            start: 1.0,
            stop: 1e6,
        },
    });
    assert_eq!(
        pxf,
        "pss1 pss fund=1G tstab=100n harms=10\n\
         pxf1 (out 0) pxf start=1 stop=1M dec=10",
    );
    assert!(!pxf.contains("isrc"), "{pxf}");

    let pstb = analysis(Analysis::SpectrePstb {
        pss: PssBase {
            pss_fundamental: 1e9,
            pss_stabilization: 1e-7,
            pss_harmonics: 10,
        },
        probe: "IPRB0".into(),
        sweep: FreqSweep {
            variation: Sweep::Dec,
            points: 10,
            start: 1.0,
            stop: 1e6,
        },
    });
    assert_eq!(
        pstb,
        "pss1 pss fund=1G tstab=100n harms=10\n\
         pstb1 pstb start=1 stop=1M dec=10 probe=iprb0",
    );
}

/// `[REF]` p.410 sweep verbatim:
/// ```text
/// swp sweep param=temp values=[-50 0 50 100 125] {
///                oppoint dc oppoint=logfile
/// }
/// ```
/// `[REF]` p.174 montecarlo verbatim:
/// `mc1 montecarlo variations=process seed=1234 numruns=200 { ... }`
/// — every parameter precedes the brace. The old codegen appended `seed=`
/// after the closing brace.
#[test]
fn spectre_sweep_and_montecarlo_blocks_match_documented_forms() {
    assert_eq!(
        analysis(Analysis::SpectreSweep {
            param: "rload".into(),
            start: 1e3,
            stop: 10e3,
            step: 1e3,
            inner: "ac1".into(),
            inner_type: SpectreAnalysis::Ac,
        }),
        "sweep1 sweep param=rload start=1k stop=10k step=1k {\n  ac1 ac\n}",
    );
    assert_eq!(
        analysis(Analysis::SpectreMonteCarlo {
            iterations: 200,
            inner: "tran1".into(),
            inner_type: SpectreAnalysis::Tran,
            seed: Some(1234),
            variations: Some("all".into()),
        }),
        "mc1 montecarlo numruns=200 seed=1234 variations=all {\n  tran1 tran\n}",
    );
    assert_eq!(
        analysis(Analysis::SpectreMonteCarlo {
            iterations: 8,
            inner: "tran1".into(),
            inner_type: SpectreAnalysis::Tran,
            seed: None,
            variations: None,
        }),
        "mc1 montecarlo numruns=8 {\n  tran1 tran\n}",
    );
}

/// `[CMP]` p.323-325: Spectre's `fourier` is a *component* placed on a node
/// pair — `Name [p] [n] [pr] [nr] fourier parameter=value ...`, verbatim
/// `four1 (1 0) fourmod harms=50`, with `fund` as instance parameter #1. It is
/// not an analysis statement and has no `signal=` parameter.
#[test]
fn spectre_fourier_is_emitted_as_a_component() {
    assert_eq!(
        analysis(Analysis::Fourier {
            fundamental: 1e3,
            outputs: vec!["V(out)".into(), "mid".into()],
            num_harmonics: Some(10),
        }),
        "four1 (out 0) fourier fund=1k harms=10\nfour2 (mid 0) fourier fund=1k harms=10",
    );
}

/// Spectre has no `trnoise` analysis. `[REF]` p.433 verbatim:
/// `tran1 tran stop=0.5u noisefmax=10G noiseseed=1` — transient noise is a
/// `tran` parameter (#92 `noisefmax`, #94 `noiseseed`) and the IR carries no
/// bandwidth, so there is nothing faithful to emit.
#[test]
fn spectre_refuses_transient_noise() {
    let r = try_analysis(&Analysis::TransientNoise {
        step: 1e-9,
        stop: 1e-6,
    });
    assert!(
        matches!(r, Err(CodeGenError::UnsupportedAnalysis { .. })),
        "{r:?}"
    );
}

/// Analyses belonging to other simulators' dialects must be refused.
#[test]
fn spectre_refuses_foreign_analyses() {
    let r = try_analysis(&Analysis::PoleZero {
        node1: "1".into(),
        node2: "0".into(),
        node3: "3".into(),
        node4: "0".into(),
        tf_type: PzTransfer::Vol,
        pz_type: PzKind::Pz,
    });
    assert!(
        matches!(r, Err(CodeGenError::UnsupportedAnalysis { .. })),
        "{r:?}"
    );
}

/// Kundert Appendix B, Table B.1 (SI, used by Spectre) vs Table B.2 (SPICE):
/// SPICE's `meg`/`g`/`t` are not Spectre scale factors; SI spells them
/// `M`/`G`/`T`. `k m u n p f` mean the same in both.
#[test]
fn spectre_numbers_use_si_scale_factors() {
    let r = |v: f64| {
        component(Component::Resistor {
            name: "1".into(),
            n1: "a".into(),
            n2: "b".into(),
            value: IrValue::Numeric { value: v },
            model: None,
            params: vec![],
        })
    };
    assert!(r(1e6).ends_with("r=1M"), "{}", r(1e6));
    assert!(r(1e9).ends_with("r=1G"), "{}", r(1e9));
    assert!(r(1e12).ends_with("r=1T"), "{}", r(1e12));
    assert!(r(1e3).ends_with("r=1k"), "{}", r(1e3));
    for v in [1e6_f64, 1e9, 1e12] {
        let s = r(v);
        assert!(
            !s.contains("meg") && !s.ends_with('g') && !s.ends_with('t'),
            "{s}"
        );
    }
}
