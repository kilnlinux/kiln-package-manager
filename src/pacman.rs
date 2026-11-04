//! Everything Kiln asks of the system goes through the `pacman` binary.
//! pacman stays the only authority on what is installed: Kiln never reads
//! /var/lib/pacman directly and never keeps its own copy of versions.
//!
//! Commands run with an argument vector (never through a shell) and with
//! `--` before package names, so a name can never become an option.
//! LC_ALL=C keeps output parseable regardless of the user's locale.

use crate::error::{Error, Result};
use std::collections::HashSet;
use std::io::IsTerminal;
use std::process::{Command, Stdio};

const PACMAN: &str = "pacman";

fn pacman() -> Command {
    let mut c = Command::new(PACMAN);
    c.env("LC_ALL", "C");
    c
}

fn spawn_err(e: std::io::Error) -> Error {
    Error::Failed(format!("cannot run {PACMAN}: {e}"))
}

/// Run pacman with captured output. Returns (exit code, stdout, stderr).
fn capture(args: &[&str], pkgs: &[String]) -> Result<(i32, String, String)> {
    let out = pacman()
        .args(args)
        .args(pkgs)
        .stdin(Stdio::null())
        .output()
        .map_err(spawn_err)?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn lines(s: &str) -> impl Iterator<Item = String> + '_ {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
}

/// Query that must exit 0; returns stdout lines.
fn query(args: &[&str]) -> Result<Vec<String>> {
    let (code, out, err) = capture(args, &[])?;
    if code != 0 {
        return crate::fail!("pacman {} failed: {}", args.join(" "), err.trim());
    }
    Ok(lines(&out).collect())
}

pub fn version() -> Result<String> {
    // The banner line looks like ` .--.   Pacman v7.0.0 - libalpm v15.0.0`.
    let out = query(&["--version"])?;
    let line = out
        .iter()
        .find_map(|l| l.find("Pacman v").map(|i| &l[i..]))
        .unwrap_or("pacman");
    Ok(line.split(" - ").next().unwrap_or(line).to_string())
}

/// The subset of `deps` not satisfied by installed packages, using
/// `pacman -T` ("deptest"). Because it is pacman's own dependency check,
/// provided names (e.g. a virtual package) count as satisfied.
pub fn unsatisfied(deps: &[String]) -> Result<Vec<String>> {
    if deps.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec!["-T", "--"];
    args.extend(deps.iter().map(String::as_str));
    let (code, out, err) = capture(&args, &[])?;
    match code {
        0 => Ok(Vec::new()),
        127 => Ok(lines(&out).collect()), // 127: some deps unsatisfied, listed on stdout
        _ => crate::fail!("pacman -T failed: {}", err.trim()),
    }
}

/// Names of all installed packages.
pub fn installed() -> Result<HashSet<String>> {
    Ok(query(&["-Qq"])?.into_iter().collect())
}

/// Names of explicitly installed packages (`pacman -Qqe`).
pub fn explicit() -> Result<Vec<String>> {
    query(&["-Qqe"])
}

/// Names of all packages in the sync databases.
pub fn available() -> Result<HashSet<String>> {
    let names: HashSet<String> = query(&["-Slq"])?.into_iter().collect();
    if names.is_empty() {
        return crate::fail!("the pacman sync databases are empty; run `sudo pacman -Syu` first");
    }
    Ok(names)
}

/// `pacman -Ss`, output straight to the terminal. Returns false if nothing matched.
pub fn search(terms: &[String]) -> Result<bool> {
    let mut args: Vec<&str> = vec!["-Ss", "--"];
    args.extend(terms.iter().map(String::as_str));
    // Not LC_ALL=C here: this output is for the user, not for parsing.
    let status = Command::new(PACMAN)
        .args(&args)
        .status()
        .map_err(spawn_err)?;
    Ok(status.success())
}

/// `pacman -Dk`: checks that every installed package's dependencies are met.
/// Returns the problems pacman reports (empty means healthy).
pub fn check_deps() -> Result<Vec<String>> {
    let (code, out, err) = capture(&["-Dk"], &[])?;
    if code == 0 {
        return Ok(Vec::new());
    }
    Ok(lines(&out).chain(lines(&err)).collect())
}

/// Effective UID == 0. Read from /proc to avoid a libc dependency for one call.
pub fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(2).map(|e| e == "0"))
        })
        .unwrap_or(false)
}

pub fn require_root(what: &str) -> Result<()> {
    if is_root() {
        Ok(())
    } else {
        crate::fail!("{what} changes the system; run it with sudo (try `--dry-run` to preview)")
    }
}

/// Run a modifying pacman transaction attached to the terminal, so pacman's
/// own summary, confirmation prompt, locking and progress all apply unchanged.
/// `yes` passes `--noconfirm`. Returns Ok(true) if pacman succeeded.
fn transaction(op: &[&str], pkgs: &[String], yes: bool) -> Result<bool> {
    if !yes && !std::io::stdin().is_terminal() {
        // pacman would read an answer from a pipe or EOF; make that explicit.
        return crate::fail!("stdin is not a terminal; pass --yes to confirm non-interactively");
    }
    let mut c = Command::new(PACMAN);
    c.args(op);
    if yes {
        c.arg("--noconfirm");
    }
    c.arg("--").args(pkgs);
    crate::ui::flush();
    Ok(c.status().map_err(spawn_err)?.success())
}

/// Install packages. `--needed` makes re-runs no-ops for anything present.
/// No `-y`: refreshing databases without upgrading (-Sy) is a partial
/// upgrade, which Arch does not support. Users run `pacman -Syu` themselves.
pub fn install(pkgs: &[String], yes: bool) -> Result<bool> {
    transaction(&["-S", "--needed"], pkgs, yes)
}

/// Remove only the requested packages (`-R`), keeping dependencies installed.
/// pacman refuses, and changes nothing, if another package still requires one.
pub fn remove(pkgs: &[String], yes: bool) -> Result<bool> {
    // Preserve dependencies that may satisfy another active profile.
    transaction(&["-R"], pkgs, yes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purge_does_not_recursively_remove_dependencies() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kiln-remove-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("pacman");
        std::fs::write(
            &fake,
            "#!/bin/sh\n[ \"$*\" = '-R --noconfirm -- parent' ]\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let old = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", &dir) };
        let result = remove(&["parent".into()], true);
        unsafe {
            match old {
                Some(path) => std::env::set_var("PATH", path),
                None => std::env::remove_var("PATH"),
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
        assert!(result.unwrap());
    }
}
