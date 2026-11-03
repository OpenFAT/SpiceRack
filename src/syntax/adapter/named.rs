//! Spectre and VACASK statements → the IR, as `import/lower/named*.rs`
//! lower them.
//!
//! Cites: `import/lower/named.rs`, `named_elements.rs`, `named_analyses.rs`.
//!
//! Contents:
//! - `Cx::declarations(&self, stmts: &[u32]) -> (HashMap<String, String>, Vec<String>)`
//! - `Cx::named_card_spine(&mut self, n: u32, here: &mut Subcircuit, scope: Scope)`

use std::collections::HashMap;

use super::{Cx, Scope};
use crate::ir::*;

impl Cx<'_> {
    /// The deck's model kinds and subcircuit names.
    pub(super) fn declarations(&self, _stmts: &[u32]) -> (HashMap<String, String>, Vec<String>) {
        (HashMap::new(), Vec::new())
    }

    /// One Spectre or VACASK statement.
    pub(super) fn named_card_spine(&mut self, n: u32, here: &mut Subcircuit, _scope: Scope) {
        self.unparsed(n, here);
    }
}
