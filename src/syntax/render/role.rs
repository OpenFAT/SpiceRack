//! A file's role → what a target writes around it: the file role rule
//! (`docs/research/transpiler-architecture.md` §2.5): a deck gets its
//! title, its `.end` and its control blocks; a library none of them, in any
//! target. The writers ask [`frame`] once per file; where the old writers'
//! rules give other bytes, a writer keeps them as a `// quirk:` beside its
//! one call, until phase 7 (`tests/fixtures/transpiler/PHASE4-STATUS.md`).
//!
//! The role is `syntax::resolve`'s ([`Role`]): the kit's answer for a kit
//! file, the same rule's for a file read alone (`resolve::role`).
//!
//! Cites: `transpiler-architecture.md` §2.5 ("A file role"); ngspice manual
//! ch.2 (the title line, `.end`), §16 (`.control`); HSPICE `.END` (ends the
//! run, in an included file too).
//!
//! Contents:
//! - `pub(super) struct Frame`
//! - `pub(super) const fn frame(role: Role) -> Frame`

use crate::syntax::resolve::Role;

/// What a target writes around a file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Frame {
    /// A title line (SPICE reads line 1 as one).
    pub(super) title: bool,
    /// The closing `.end`.
    pub(super) end: bool,
    /// Control blocks: ngspice's `.control` (its `pre_osdi` loads, the run
    /// a `.meas` needs).
    pub(super) control: bool,
}

/// What `role` says a file carries.
///
/// # Panics
/// For a Verilog-A source or an unread file, which no writer writes.
pub(super) const fn frame(role: Role) -> Frame {
    match role {
        Role::Deck => Frame {
            title: true,
            end: true,
            control: true,
        },
        Role::Library => Frame {
            title: false,
            end: false,
            control: false,
        },
        Role::VerilogA | Role::Unread(_) => panic!("frame: a file the kit read"),
    }
}
