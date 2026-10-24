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
