//! A card the old grammar refused → `Raw` or `Unparsed`, as
//! the old SPICE grammar sorted them: a dot card whose word is a `rawkind`
//! (or an overlay's own raw word) was `Raw`, any other `Unparsed`.
//!
//! Cites: the old SPICE grammar's `Raw`, `rawkind`, `Unparsed`; the old LTspice grammar's `Raw_lt`;
//! the old Xyce grammar's `Raw_xy`.
//!
//! Contents:
//! - `Rd::refused(&self, n: u32) -> Kept`
//! - `Rd::dialect_raw(&self, w: &str) -> bool`

use super::{Kept, Rd};
use crate::syntax::Dialect;
use crate::syntax::token::Tag as T;

/// the old SPICE grammar's `rawkind`: the dot words whose card, when no rule reads it,
/// is `Raw`; any other dot card no rule reads is `Unparsed`.
const RAWKIND: &[&str] = &[
    "acphasenoise",
    "osdi_include",
    "appendmodel",
    "temperature",
    "montecarlo",
    "phasenoise",
    "tran_noise",
    "trannoise",
    "unprotect",
    "envelope",
    "ptdnoise",
    "pre_osdi",
    "acmatch",
    "dcmatch",
    "hbnoise",
    "snnoise",
    "measure",
    "biaschk",
    "stimuli",
    "connect",
    "protect",
    "options",
    "nodeset",
    "include",
    "verilog",
    "dcsens",
    "pnoise",
    "jitter",
    "sample",
    "unprot",
    "option",
    "dcvolt",
    "global",
    "dcinc",
    "disto",
    "envlp",
    "hbosc",
    "hblin",
    "matex",
    "noise",
    "snosc",
    "check",
    "power",
    "store",
    "mosra",
    "print",
    "probe",
    "graph",
    "width",
    "title",
    "model",
    "acxf",
    "dcxf",
    "four",
    "hbac",
    "hbxf",
    "lstb",
    "qpss",
    "sens",
    "snac",
    "snxf",
    "tran",
    "meas",
    "dout",
    "stim",
    "plot",
    "prot",
    "step",
    "endc",
    "opts",
    "temp",
    "save",
    "fft",
    "lin",
    "net",
    "pac",
    "pss",
    "pxf",
    "stb",
    "pat",
    "opt",
    "hdl",
    "ac",
    "dc",
    "hb",
    "mc",
    "op",
    "pz",
    "sn",
    "sp",
    "tf",
    "ic",
];

impl<'a> Rd<'a> {
    /// quirk: the old SPICE grammar's `Raw <- '.' rawkind !namechar rest /
    /// ends rest`: a dot card no rule reads is `Raw` when its word is a `rawkind` (or
    /// `.ends`/`.eom`), else `Unparsed` (and so are non-dot lines).
    pub(in crate::syntax::render) fn refused(&self, n: u32) -> Kept {
        // The dot word as the old SPICE grammar's `!namechar` reads it: the head token
        // and the name bytes touching it (`.probe/csd` is no `.probe`).
        let first = self.main(n);
        let last = self.line_last(first);
        let mut end = first;
        while end < last
            && self.t.end(end) == self.t.start(end + 1)
            && !matches!(
                self.ttag(end + 1),
                T::Comma
                    | T::Equal
                    | T::EqualEqual
                    | T::BangEqual
                    | T::LessEqual
                    | T::GreaterEqual
                    | T::LParen
                    | T::RParen
                    | T::Quote
                    | T::DoubleQuote
                    | T::String
                    | T::LBrace
                    | T::RBrace
            )
        {
            end += 1;
        }
        let word = self.span(first, end).to_ascii_lowercase();
        let Some(w) = word.strip_prefix('.') else {
            return Kept::Unparsed;
        };
        if w == "ends" || w == "eom" || self.dialect_raw(w) || RAWKIND.contains(&w) {
            Kept::Raw
        } else {
            Kept::Unparsed
        }
    }

    /// quirk: whether `w` is one of the old dialect overlays' own raw dot
    /// words (the old LTspice grammar's `Raw_lt`, the old Xyce grammar's
    /// `Raw_xy`), listed below: such a card the grammar read and kept raw.
    pub(in crate::syntax::render) fn dialect_raw(&self, w: &str) -> bool {
        const LT: &[&str] = &[
            "endmachine",
            "savestate",
            "loadstate",
            "softstart",
            "backanno",
            "savebias",
            "loadbias",
            "machine",
            "fourier",
            "output",
            "ferret",
            "state",
            "wave",
            "rule",
            "endl",
            "fra",
        ];
        const XY: &[&str] = &[
            "embeddedsampling",
            "global_param",
            "temperature",
            "preprocess",
            "objective",
            "sampling",
            "initcond",
            "result",
            "mpde",
            "dcop",
            "temp",
            "save",
            "sens",
            "pce",
            "mor",
            "rol",
        ];
        match self.d {
            Dialect::Ltspice => LT.contains(&w),
            Dialect::Xyce => XY.contains(&w),
            _ => false,
        }
    }
}

// ---- private ----
