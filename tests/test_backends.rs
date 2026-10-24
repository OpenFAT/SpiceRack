use spicerack::circuit::*;

#[test]
fn test_available_backends_api() {
    let backends = spicerack::simulation::CircuitSimulator::available_backends();
    // Returns strings, should not panic
    for b in &backends {
        assert!(!b.is_empty());
    }
}

#[test]
fn test_backend_override_creates_simulator() {
    let mut c = Circuit::new("backend_test");
    c.v("dd", "vdd", Node::Ground, 3.3);

    // All these should create simulators without panicking
    let _ = c.simulator_with_backend("ngspice");
    let _ = c.simulator_with_backend("ltspice");
    let _ = c.simulator_with_backend("vacask");
    let _ = c.simulator_with_backend("spectre");
}

// ── Circuit feature detection tests ──

#[test]
fn test_has_xspice_false_when_no_a_elements() {
    let mut c = Circuit::new("no_xspice");
    c.v("1", "in", Node::Ground, 1.0);
    c.r("1", "in", "out", 1e3);
    assert!(!c.has_xspice());
}

#[test]
fn test_has_xspice_true_when_a_element_present() {
    let mut c = Circuit::new("with_xspice");
    c.v("1", "in", Node::Ground, 1.0);
    c.a("1", vec!["[in]".into(), "[out]".into()], "d_inverter");
    assert!(c.has_xspice());
}

#[test]
fn test_has_osdi_false_when_no_loads() {
    let mut c = Circuit::new("no_osdi");
    c.v("1", "in", Node::Ground, 1.0);
    assert!(!c.has_osdi());
}

#[test]
fn test_has_osdi_true_when_osdi_loaded() {
    let mut c = Circuit::new("with_osdi");
    c.osdi("bsim4.osdi");
    c.v("1", "in", Node::Ground, 1.0);
    assert!(c.has_osdi());
}

#[test]
fn test_has_osdi_multiple_loads() {
    let mut c = Circuit::new("multi_osdi");
    c.osdi("bsim4.osdi");
    c.osdi("psp103.osdi");
    assert!(c.has_osdi());
    assert_eq!(c.osdi_loads().len(), 2);
}

#[test]
fn test_circuit_features_both() {
    let mut c = Circuit::new("both_features");
    c.osdi("bsim4.osdi");
    c.a("1", vec!["[in]".into(), "[out]".into()], "d_inverter");
    assert!(c.has_xspice());
    assert!(c.has_osdi());
}

#[test]
fn test_osdi_emitted_in_netlist() {
    let mut c = Circuit::new("osdi_netlist");
    c.osdi("mymodel.osdi");
    c.v("1", "in", Node::Ground, 1.0);
    let netlist = c.to_string();
    assert!(netlist.contains("pre_osdi mymodel.osdi"), "OSDI load missing from netlist");
    // pre_osdi must appear before .title
    let osdi_pos = netlist.find("pre_osdi").unwrap();
    let title_pos = netlist.find(".title").unwrap();
    assert!(osdi_pos < title_pos, "pre_osdi must appear before .title");
}

// ── Circuit .control block detection tests ──

#[test]
fn test_has_control_blocks_false_by_default() {
    let c = Circuit::new("no_control");
    assert!(!c.has_control_blocks());
}

#[test]
fn test_has_control_blocks_true_when_present() {
    let mut c = Circuit::new("with_control");
    c.raw_spice(".control");
    c.raw_spice("run");
    c.raw_spice(".endc");
    assert!(c.has_control_blocks());
}

#[test]
fn test_has_control_blocks_case_insensitive() {
    let mut c = Circuit::new("control_case");
    c.raw_spice(".CONTROL");
    assert!(c.has_control_blocks());
}

// ── Laplace B-source detection tests ──

#[test]
fn test_has_laplace_sources_false_by_default() {
    let c = Circuit::new("no_laplace");
    assert!(!c.has_laplace_sources());
}

#[test]
fn test_has_laplace_sources_false_for_normal_bsource() {
    let mut c = Circuit::new("normal_bsource");
    c.bv("1", "out", Node::Ground, "V(in)*2");
    assert!(!c.has_laplace_sources());
}

#[test]
fn test_has_laplace_sources_true_for_bv() {
    let mut c = Circuit::new("laplace_bv");
    c.bv("1", "out", Node::Ground, "Laplace(V(in), 1/(1+s/6.28e3))");
    assert!(c.has_laplace_sources());
}

#[test]
fn test_has_laplace_sources_true_for_bi() {
    let mut c = Circuit::new("laplace_bi");
    c.bi("1", "out", Node::Ground, "Laplace(I(V1), 1/(1+s/1e6))");
    assert!(c.has_laplace_sources());
}

// ── New analysis netlist generation tests ──

#[test]
fn test_pss_netlist() {
    let mut c = Circuit::new("pss_test");
    c.v("dd", "vdd", Node::Ground, 3.3);

    let sim = c.simulator();
    let netlist = sim.build_netlist_for_test(".pss 1e9 100e-9 v(out) 128 10");
    assert!(netlist.contains(".pss 1e9 100e-9 v(out) 128 10"));
}

#[test]
fn test_sparam_netlist() {
    let mut c = Circuit::new("sp_test");
    c.v("dd", "vdd", Node::Ground, 3.3);

    let sim = c.simulator();
    let netlist = sim.build_netlist_for_test(".sp dec 10 1e6 1e10");
    assert!(netlist.contains(".sp dec 10 1e6 1e10"));
}

#[test]
fn test_hb_netlist() {
    let mut c = Circuit::new("hb_test");
    c.v("dd", "vdd", Node::Ground, 3.3);

    let sim = c.simulator();
    let netlist = sim.build_netlist_for_test(".HB 1000000000");
    assert!(netlist.contains(".HB 1000000000"));
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
