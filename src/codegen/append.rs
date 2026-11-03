//! The `w!` macro every codegen writer appends with.

/// `write!` into a `String`: every writer here appends to the one deck
/// buffer instead of formatting a `String` per card and copying it.
macro_rules! w {
    ($out:expr, $($fmt:tt)*) => {{
        use std::fmt::Write as _;
        $out.write_fmt(format_args!($($fmt)*)).expect("writing to a String cannot fail")
    }};
}
