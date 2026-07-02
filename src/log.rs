pub(crate) const PROG_NAME: &str = "build";

macro_rules! blog {
    (@, $color:ident, $label:expr, $sep:expr, target = $target:expr, $($inp:tt)*) => {{
        use yansi::Paint as _;
        eprintln!("{} [{}] {}{}{}", $crate::log::PROG_NAME.$color(), ($target).bright_blue(), $label, $sep, format_args!($($inp)*))
    }};
    (@, $color:ident, $label:expr, $sep:expr, $($inp:tt)*) => {{
        use yansi::Paint as _;
        eprintln!("{} {}{}{}", $crate::log::PROG_NAME.$color(), $label, $sep, format_args!($($inp)*))
    }};
    (error, $($inp:tt)*) => {{ blog!(@, red, { use yansi::Paint as _; "Error".red().bold() }, ": ", $($inp)*) }};
    (warn,  $($inp:tt)*) => {{ blog!(@, yellow, { use yansi::Paint as _; "Warn".yellow().bold() }, ": ", $($inp)*) }};
    // Don't show severity for the default case (info)
    (info,  $($inp:tt)*) => {{ blog!(@, green, "", "", $($inp)*) }};
}

pub(crate) use blog;
