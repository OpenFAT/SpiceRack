//! Detection for Cadence Spectre PSF (Parameter Storage Format) binary files.
//!
//! PSF is what Spectre writes when it is not asked for another format. This
//! module **detects** PSF and nothing more — see [`parse_psf`] for why, and for
//! the framing a future implementation needs.
//!
//! The Spectre backend passes `-format nutbin`, so Spectre emits Nutmeg and
//! [`crate::rawfile::parse_raw`] reads the results.

use crate::result::RawData;

/// PSF's magic, stored in the file's trailer rather than at its head.
const PSF_MAGIC: &[u8] = b"Clarissa";

#[derive(Debug, thiserror::Error)]
pub enum PsfError {
    #[error("Bad magic: expected 'Clarissa' in the final 12 bytes")]
    BadMagic,
    #[error(
        "PSF binary parsing is not implemented; run Spectre with -format nutbin \
         (the backend does) and read the Nutmeg output instead"
    )]
    Unsupported,
}

/// Parse a PSF binary file.
///
/// **Not implemented, deliberately.** An earlier version of this function
/// scanned forward from the head of the file looking for section markers. That
/// is not how PSF is laid out, so it never read a file Spectre produced; it was
/// also unreachable, because [`is_psf`] checked for the magic in the wrong
/// place and always answered `false`.
///
/// Implementing it properly needs the section-content encodings, and those
/// cannot be verified without a real Spectre PSF file — which needs a Cadence
/// licence. Rather than ship a parser nobody can check, the Spectre backend
/// passes `-format nutbin` so Spectre writes Nutmeg, which
/// [`crate::rawfile::parse_raw`] reads correctly.
///
/// The framing, for whoever implements this (source: libpsf `PSFFile`):
///
/// ```text
/// [ section data ... ][ TOC ][ "Clarissa" (8 bytes) ][ datasize: u32 ]
/// ```
/// - `datasize` is the final 4 bytes.
/// - `nsections = (size - datasize - 12) / 8`.
/// - The TOC starts at `size - 12 - nsections * 8`; each entry is
///   `(section_number: u32, offset: u32)`.
/// - Sections are reached through that table, never by scanning forward.
pub fn parse_psf(data: &[u8]) -> Result<RawData, PsfError> {
    if !is_psf(data) {
        return Err(PsfError::BadMagic);
    }
    Err(PsfError::Unsupported)
}

/// True if `data` is a PSF binary file.
///
/// PSF carries its magic in the **last 12 bytes** — `"Clarissa"` followed by a
/// 4-byte data size — not at the head. libpsf validates by seeking to `-12`
/// from the end and comparing 8 bytes.
pub fn is_psf(data: &[u8]) -> bool {
    data.len() >= 12 && &data[data.len() - 12..data.len() - 4] == PSF_MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sections + TOC, then the trailer: magic and a 4-byte data size.
    fn spec_shaped_psf() -> Vec<u8> {
        [vec![0u8; 80], PSF_MAGIC.to_vec(), 64u32.to_be_bytes().to_vec()].concat()
    }

    #[test]
    fn detects_a_spec_correct_file() {
        assert!(is_psf(&spec_shaped_psf()));
    }

    #[test]
    fn rejects_nutmeg_and_short_input() {
        assert!(!is_psf(b"Title: test\nPlotname:"));
        assert!(!is_psf(b"short"));
        assert!(!is_psf(b""));
    }

    #[test]
    fn parsing_refuses_rather_than_guessing() {
        assert!(matches!(parse_psf(&spec_shaped_psf()), Err(PsfError::Unsupported)));
        assert!(matches!(parse_psf(b"Title: not psf"), Err(PsfError::BadMagic)));
    }
}
