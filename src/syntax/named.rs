//! A Spectre or VACASK statement head → its node: a keyword statement, a
//! block opener or closer, or an instance led by its name.
//!
//! Cites: `grammar/spectre.peg`, `grammar/vacask.peg`.
//!
//! Contents:
//! - `Parser::named(&mut self, in_tag: Tag) -> Head`

use crate::syntax::ast::{NONE, Tag};
use crate::syntax::parse::{Head, Parser};
use crate::syntax::token::Tag as T;

impl Parser<'_, '_> {
    /// The Spectre or VACASK statement at the cursor, inside `in_tag`.
    pub(super) fn named(&mut self, _in_tag: Tag) -> Head {
        let first = self.i;
        if self.t.tags()[first as usize] == T::Title {
            self.i = self.end;
            return Head::Flat(self.ast.push(Tag::Title, first, NONE, NONE));
        }
        Head::Flat(self.verbatim(first))
    }
}
