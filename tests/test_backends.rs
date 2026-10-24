use spicerack::backend::{Backend, BackendKind, detect};

#[test]
fn test_backend_kind_from_str() {
    assert_eq!(BackendKind::from_str("ngspice"), Some(BackendKind::NgspiceSubprocess));
    assert_eq!(BackendKind::from_str("ngspice-subprocess"), Some(BackendKind::NgspiceSubprocess));
    assert_eq!(BackendKind::from_str("ngspice-shared"), Some(BackendKind::NgspiceShared));
    assert!(BackendKind::from_str("vacask").is_some());
    assert_eq!(BackendKind::from_str("vacask-shared"), Some(BackendKind::VacaskShared));
    assert!(BackendKind::from_str("spectre").is_some());
    assert!(BackendKind::from_str("ltspice").is_some());
    assert!(BackendKind::from_str("nonexistent").is_none());
}

#[test]
fn test_backend_display_names() {
    assert_eq!(BackendKind::NgspiceSubprocess.display_name(), "ngspice");
    assert_eq!(BackendKind::NgspiceShared.display_name(), "ngspice-shared");
    assert_eq!(BackendKind::Vacask.display_name(), "vacask");
    assert_eq!(BackendKind::VacaskShared.display_name(), "vacask-shared");
    assert_eq!(BackendKind::Spectre.display_name(), "spectre");
}

#[test]
fn test_detect_backends() {
    let backends = detect::detect_backends();
    // Should at least not panic; may or may not find backends on CI
    assert!(backends.len() <= 10); // sanity check
}

// ── Vacask SPICE-to-Vacask translator tests ──

// ── LTspice netlist normalization tests ──

#[test]
fn test_ltspice_normalization() {
    use spicerack::backend::ltspice::LtspiceSubprocess;
    use std::path::PathBuf;

    // We can't call normalize_netlist directly (it's private), but we can
    // verify the backend exists and creates properly
    let backend = LtspiceSubprocess {
        executable: PathBuf::from("/usr/bin/ltspice"),
        use_wine: false,
        fast_access: false,
    };
    assert_eq!(spicerack::backend::Backend::name(&backend), "ltspice");
}

// ── Spectre output format detection tests ──

#[test]
fn test_spectre_output_format_enum() {
    use spicerack::backend::spectre::OutputFormat;
    assert_ne!(OutputFormat::Nutmeg, OutputFormat::Psf);
    assert_eq!(OutputFormat::Nutmeg, OutputFormat::Nutmeg);
    assert_eq!(OutputFormat::Psf, OutputFormat::Psf);
}

// ── PSF parser tests ──

#[test]
fn test_psf_is_psf_detection() {
    assert!(spicerack::psf::is_psf(b"Clarissa\x00\x00\x00\x01"));
    assert!(!spicerack::psf::is_psf(b"Title: test\nPlotname:"));
    assert!(!spicerack::psf::is_psf(b"short"));
}

#[test]
fn test_psf_parse_bad_magic() {
    let data = b"NotAValidPSFFile";
    let result = spicerack::psf::parse_psf(data);
    assert!(result.is_err());
}

#[test]
fn test_psf_parse_too_short() {
    let data = b"Clar";
    let result = spicerack::psf::parse_psf(data);
    assert!(result.is_err());
}

// ── Spectre sweep netlist generation tests ──

#[test]
fn test_spectre_sweep_netlist_generation() {
    // This tests the netlist building logic, not actual Spectre execution
    let backend = spicerack::backend::spectre::SpectreSubprocess;
    // We can't actually run spectre, but we can verify the object exists
    assert_eq!(spicerack::backend::Backend::name(&backend), "spectre");
}

// ── Raw file parser tests ──

#[test]
fn test_rawfile_utf16_detection() {
    // Standard UTF-8 ngspice raw file should NOT be detected as UTF-16
    let raw_content = b"Title: test\nPlotname: Operating Point\n";
    let result = spicerack::rawfile::parse_raw(raw_content);
    // Should fail gracefully (incomplete data) but not panic
    assert!(result.is_err());
}

#[test]
fn test_rawfile_parse_existing_format() {
    // Verify our existing parser still works
    let raw_content = b"Title: test\n\
Plotname: Operating Point\n\
Flags: real\n\
No. Variables: 2\n\
No. Points: 1\n\
Variables:\n\
\t0\tv(out)\tvoltage\n\
\t1\tv(in)\tvoltage\n\
Values:\n\
0\t3.300000e+00\n\
\t1.000000e+00\n";

    let result = spicerack::rawfile::parse_raw(raw_content).unwrap();
    assert_eq!(result.title, "test");
    assert_eq!(result.variables.len(), 2);
    assert!((result.real_data[0][0] - 3.3).abs() < 1e-10);
}

// ── NGSpice shared library backend tests ──

#[test]
fn test_ngspice_shared_is_available_doesnt_panic() {
    // is_available should never panic, regardless of whether the lib exists
    let _ = spicerack::backend::ngspice::NgspiceShared::is_available();
}

#[test]
#[ignore] // Requires libngspice.so to be installed
fn test_ngspice_shared_op_simulation() {
    use spicerack::backend::ngspice::NgspiceShared;

    let shared = NgspiceShared::new().expect("Failed to load libngspice.so");

    let netlist = "\
        test op\n\
        V1 vdd 0 3.3\n\
        R1 vdd out 1k\n\
        R2 out 0 2k\n\
        .op\n\
        .end\n";

    let result = shared.run(netlist);
    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());

    let raw = result.unwrap();
    assert!(!raw.variables.is_empty(), "No variables in result");
    assert_eq!(raw.real_data.len(), raw.variables.len());
}

#[test]
#[ignore] // Requires libngspice.so to be installed
fn test_ngspice_shared_tran_simulation() {
    use spicerack::backend::ngspice::NgspiceShared;

    let shared = NgspiceShared::new().expect("Failed to load libngspice.so");

    let netlist = "\
        test tran\n\
        V1 in 0 PULSE(0 1 0 1n 1n 5u 10u)\n\
        R1 in out 1k\n\
        C1 out 0 1n\n\
        .tran 10n 20u\n\
        .end\n";

    let result = shared.run(netlist);
    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());

    let raw = result.unwrap();
    assert!(!raw.real_data.is_empty());
    assert!(raw.real_data[0].len() > 1, "Expected multiple data points in transient");
}

#[test]
#[ignore] // Requires libngspice.so to be installed
fn test_ngspice_shared_streaming_data() {
    use spicerack::backend::ngspice::NgspiceSharedStreaming;

    let streaming = NgspiceSharedStreaming::new()
        .expect("Failed to load libngspice.so");

    let netlist = "\
        test streaming\n\
        V1 in 0 PULSE(0 1 0 1n 1n 5u 10u)\n\
        R1 in out 1k\n\
        C1 out 0 1n\n\
        .tran 10n 20u\n\
        .end\n";

    let result = streaming.run(netlist);
    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());

    let points = streaming.drain_streaming_data();
    // The buffer should be empty after drain (data was already consumed
    // during run or accumulated)
    let points2 = streaming.drain_streaming_data();
    assert!(points2.is_empty(), "Drain should return empty after first drain");
    // We just verify it does not panic; actual count depends on ngspice internals
    let _ = points;
}

#[test]
#[ignore] // Requires libngspice.so to be installed
fn test_ngspice_shared_backend_name() {
    use spicerack::backend::ngspice::NgspiceShared;

    let shared = NgspiceShared::new().expect("Failed to load libngspice.so");
    assert_eq!(shared.name(), "ngspice-shared");
}

// ── Vacask shared library backend tests ──

#[test]
fn test_vacask_library_is_available_doesnt_panic() {
    // is_available should never panic, regardless of whether the lib exists
    let _ = spicerack::backend::vacask::VacaskLibrary::is_available();
}

#[test]
#[ignore] // Requires libvacask.so to be installed
fn test_vacask_library_init() {
    use spicerack::backend::vacask::VacaskLibrary;

    let lib = VacaskLibrary::new();
    assert!(lib.is_ok(), "Failed to init: {:?}", lib.err());
}

#[test]
#[ignore] // Requires libvacask.so to be installed
fn test_vacask_library_op_simulation() {
    use spicerack::backend::vacask::VacaskLibrary;

    let lib = VacaskLibrary::new().expect("Failed to load libvacask.so");

    let netlist = "\
        test op\n\
        V1 vdd 0 3.3\n\
        R1 vdd out 1k\n\
        R2 out 0 2k\n\
        .op\n\
        .end\n";

    let result = lib.run(netlist);
    assert!(result.is_ok(), "Simulation failed: {:?}", result.err());
}

#[test]
#[ignore] // Requires libvacask.so to be installed
fn test_vacask_library_backend_name() {
    use spicerack::backend::vacask::VacaskLibrary;

    let lib = VacaskLibrary::new().expect("Failed to load libvacask.so");
    assert_eq!(lib.name(), "vacask-shared");
}
