//! Formatted text -> the end of the one deck buffer every codegen writer
//! appends to. Cites: none.
//!
//! Contents: `w!(out, fmt, args...)` (crate-private macro, in scope in every
//! module declared after `#[macro_use] mod append;`).

/// `write!` into a `String`: every writer here appends to the one deck
/// buffer instead of formatting a `String` per card and copying it.
macro_rules! w {
    ($out:expr, $($fmt:tt)*) => {{
        use std::fmt::Write as _;
        $out.write_fmt(format_args!($($fmt)*)).expect("writing to a String cannot fail")
    }};
}
