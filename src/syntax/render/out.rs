//! The output stream every writer writes through: one [`Out::token`] choke
//! point, a closed [`Space`] vocabulary for what follows a token, and the
//! indentation, which the stream applies at a line's start so no writer
//! counts columns (Zig `Ast/Render.zig`'s `renderToken` and
//! `AutoIndentingStream`).
//!
//! Cites: `compiler-architecture-reference.md` §4.2 (`renderToken`,
//! `Space`), §4.9 (`AutoIndentingStream`).
//!
//! Contents:
//! - `pub(super) struct Out`, `pub(super) enum Space`
//! - `Out::new(capacity: usize) -> Out`, `Out::token(&mut self, text: &str, then: Space)`,
//!   `Out::indent(&mut self)`, `Out::dedent(&mut self)`, `Out::pop(&mut self)`,
//!   `Out::finish(self) -> String`

/// The deck text being written.
pub(super) struct Out {
    text: String,
    /// The indentation levels open.
    depth: u16,
    /// The spaces per indentation level.
    width: u16,
    /// Nothing has been written on the current line yet.
    line_empty: bool,
}

/// What follows a token.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Space {
    /// Nothing: the next token touches it.
    None,
    /// One blank.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the named writer spaces tokens")
    )]
    Blank,
    /// A line end.
    Line,
}

impl Out {
    /// An empty stream with room for `capacity` bytes.
    pub(super) fn new(capacity: usize) -> Out {
        Out {
            text: String::with_capacity(capacity),
            depth: 0,
            width: 2,
            line_empty: true,
        }
    }

    /// Writes `text`, after the line's indentation when it starts one, then
    /// `then`. Every byte of a deck goes through here; `text` holds no line
    /// end but where a statement is kept as its lines.
    pub(super) fn token(&mut self, text: &str, then: Space) {
        if self.line_empty && !text.is_empty() {
            let pad = usize::from(self.depth * self.width);
            self.text.extend(std::iter::repeat_n(' ', pad));
        }
        self.text.push_str(text);
        self.line_empty &= text.is_empty();
        match then {
            Space::None => {}
            Space::Blank => {
                self.text.push(' ');
                self.line_empty = false;
            }
            Space::Line => {
                self.text.push('\n');
                self.line_empty = true;
            }
        }
    }

    /// Opens an indentation level for the lines after this one.
    pub(super) fn indent(&mut self) {
        self.depth += 1;
    }

    /// Closes the innermost indentation level.
    pub(super) fn dedent(&mut self) {
        assert!(self.depth > 0, "a level is open");
        self.depth -= 1;
    }

    /// Takes back the last byte written (a deck that ends without its
    /// last line end).
    pub(super) fn pop(&mut self) {
        let last = self.text.pop();
        self.line_empty = last == Some('\n') || self.text.is_empty() || self.text.ends_with('\n');
    }

    /// The text written.
    pub(super) fn finish(self) -> String {
        assert_eq!(self.depth, 0, "every indentation level closed");
        self.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_take_their_space_after_and_lines_their_indentation() {
        let mut o = Out::new(0);
        o.token(".subckt", Space::Blank);
        o.token("a", Space::Line);
        o.indent();
        o.token("r1", Space::Blank);
        o.token("(a b)", Space::Line);
        o.dedent();
        o.token(".ends", Space::Line);
        o.pop();
        assert_eq!(o.finish(), ".subckt a\n  r1 (a b)\n.ends");
    }
}
