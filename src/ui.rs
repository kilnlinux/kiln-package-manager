//! Terminal output helpers. Color only when stdout is a terminal, TERM is not
//! "dumb", and NO_COLOR (https://no-color.org) is unset.

use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

fn color_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none()
            && std::env::var("TERM").map_or(true, |t| t != "dumb")
    })
}

fn paint(code: &str, s: &str) -> String {
    if color_enabled() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Kiln orange, used sparingly for names and headings.
pub fn accent(s: &str) -> String {
    paint("38;2;164;71;31", s)
}
pub fn bold(s: &str) -> String {
    paint("1", s)
}
pub fn dim(s: &str) -> String {
    paint("2", s)
}
pub fn good(s: &str) -> String {
    paint("32", s)
}
pub fn warn(s: &str) -> String {
    paint("33", s)
}
pub fn bad(s: &str) -> String {
    paint("31", s)
}

/// Print a heading line: `:: text`.
pub fn heading(s: &str) {
    println!("{} {}", accent("::"), bold(s));
}

/// Print a package list, indented, one per line.
pub fn list(items: &[String]) {
    for i in items {
        println!("     {i}");
    }
}

/// Flush stdout before handing the terminal to pacman so output stays ordered.
pub fn flush() {
    let _ = std::io::stdout().flush();
}
