//! Profile manifests: parsing, validation and lookup on disk.
//!
//! A manifest is a TOML file named `<name>.toml` whose `[profile] name`
//! matches the file name. Unknown keys are errors, so a typo such as
//! `[pakages]` fails loudly instead of silently doing nothing.

use crate::error::{Error, Result, io_err};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub profile: Meta,
    #[serde(default)]
    pub packages: Packages,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// Package sources. Only `official` (packages from the repositories in
/// /etc/pacman.conf) exists today. A new source is a new optional field here
/// plus the code that handles it; existing manifests keep working.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    #[serde(default)]
    pub official: Vec<String>,
}

/// Profile names become file names and state keys: keep them boring.
pub fn valid_profile_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// pacman package-name rules: alphanumerics and `@._+-`, not starting with
/// `-` or `.`. The leading-hyphen rule also keeps names from being read as
/// pacman options (we pass `--` as well).
pub fn valid_package_name(n: &str) -> bool {
    !n.is_empty()
        && !n.starts_with('-')
        && !n.starts_with('.')
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
}

impl Manifest {
    pub fn parse(text: &str, origin: &Path) -> Result<Manifest> {
        let m: Manifest = toml::from_str(text)
            .map_err(|e| Error::Failed(format!("{}: {}", origin.display(), e.message())))?;
        m.validate(origin)?;
        Ok(m)
    }

    fn validate(&self, origin: &Path) -> Result<()> {
        let at = origin.display();
        let name = &self.profile.name;
        if !valid_profile_name(name) {
            return crate::fail!("{at}: invalid profile name {name:?} (use a-z, 0-9, '-', '_')");
        }
        let stem = origin.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem != name {
            return crate::fail!(
                "{at}: profile name {name:?} must match the file name ({stem}.toml)"
            );
        }
        let mut seen = HashSet::new();
        for p in &self.packages.official {
            if !valid_package_name(p) {
                return crate::fail!("{at}: invalid package name {p:?}");
            }
            if !seen.insert(p) {
                return crate::fail!("{at}: package {p:?} is listed twice");
            }
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Manifest> {
        let text = std::fs::read_to_string(path).map_err(io_err("reading", path))?;
        Manifest::parse(&text, path)
    }
}

/// Directories searched for `<name>.toml`, highest priority first, so a user
/// copy overrides an admin copy, which overrides the one Kiln ships.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = user_dir() {
        dirs.push(d);
    }
    dirs.push(PathBuf::from("/etc/kiln/profiles"));
    dirs.push(PathBuf::from("/usr/share/kiln/profiles"));
    dirs
}

/// The invoking user's profile directory, `~/.config/kiln/profiles`.
/// Under sudo this is still the *invoking* user's directory, so
/// `sudo kiln apply mine` finds the profile the user wrote.
pub fn user_dir() -> Option<PathBuf> {
    if let (true, Ok(user)) = (crate::pacman::is_root(), std::env::var("SUDO_USER")) {
        return home_of(&user).map(|h| h.join(".config/kiln/profiles"));
    }
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(base.join("kiln/profiles"))
}

/// Home directory from /etc/passwd (field 6).
fn home_of(user: &str) -> Option<PathBuf> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd
        .lines()
        .map(|l| l.split(':').collect::<Vec<_>>())
        .find(|f| f.len() >= 6 && f[0] == user)
        .map(|f| PathBuf::from(f[5]))
}

/// Turn a CLI argument into a manifest path. Anything containing `/` or
/// ending in `.toml` is a path; otherwise it is a profile name to search for.
pub fn resolve(arg: &str) -> Result<PathBuf> {
    if arg.contains('/') || arg.ends_with(".toml") {
        return Ok(PathBuf::from(arg));
    }
    if !valid_profile_name(arg) {
        return Err(Error::Usage(format!("invalid profile name {arg:?}")));
    }
    let dirs = search_dirs();
    for d in &dirs {
        let p = d.join(format!("{arg}.toml"));
        if p.is_file() {
            return Ok(p);
        }
    }
    let searched: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
    crate::fail!(
        "profile {arg:?} not found (searched {})",
        searched.join(", ")
    )
}

/// Every discoverable profile, first match per name, sorted by name.
pub fn discover() -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for d in search_dirs() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let Some(stem) = p.file_stem().and_then(|s| s.to_str()).map(String::from) else {
                continue;
            };
            if p.extension().is_some_and(|x| x == "toml") && !found.iter().any(|(n, _)| *n == stem)
            {
                found.push((stem, p));
            }
        }
    }
    found.sort();
    found
}

/// Template written by `kiln profile create`.
pub fn template(name: &str) -> String {
    format!(
        "[profile]\nname = \"{name}\"\ndescription = \"\"\n\n[packages]\n\
         # Package names from the official repositories (see `kiln search`).\n\
         official = [\n]\n"
    )
}

/// Manifest text for `kiln export`.
pub fn render(name: &str, description: &str, packages: Vec<String>) -> String {
    let m = Manifest {
        profile: Meta {
            name: name.to_string(),
            description: description.to_string(),
        },
        packages: Packages { official: packages },
    };
    toml::to_string(&m).expect("manifest serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(name: &str, text: &str) -> Result<Manifest> {
        Manifest::parse(text, Path::new(&format!("/x/{name}.toml")))
    }

    #[test]
    fn parses_valid_manifest() {
        let m = parse(
            "dev",
            "[profile]\nname='dev'\n[packages]\nofficial=['git','gcc']",
        )
        .unwrap();
        assert_eq!(m.packages.official, ["git", "gcc"]);
    }

    #[test]
    fn packages_table_is_optional() {
        assert!(
            parse("empty", "[profile]\nname='empty'")
                .unwrap()
                .packages
                .official
                .is_empty()
        );
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(parse("a", "[profile]\nname='a'\n[pakages]\nofficial=[]").is_err());
        assert!(parse("a", "[profile]\nname='a'\n[packages]\naur=['x']").is_err());
    }

    #[test]
    fn rejects_name_mismatch_and_bad_names() {
        assert!(parse("a", "[profile]\nname='b'").is_err());
        assert!(parse("A", "[profile]\nname='A'").is_err());
        assert!(
            parse(
                "a",
                "[profile]\nname='a'\n[packages]\nofficial=['--root=/']"
            )
            .is_err()
        );
        assert!(parse("a", "[profile]\nname='a'\n[packages]\nofficial=['a b']").is_err());
        assert!(
            parse(
                "a",
                "[profile]\nname='a'\n[packages]\nofficial=['git','git']"
            )
            .is_err()
        );
    }

    #[test]
    fn package_name_rules() {
        for ok in [
            "git",
            "lib32-glibc",
            "gtk+",
            "python3.12",
            "c++utilities",
            "foo@bar",
        ] {
            assert!(valid_package_name(ok), "{ok}");
        }
        for bad in ["", "-x", ".x", "a/b", "a;b", "a b", "ä"] {
            assert!(!valid_package_name(bad), "{bad}");
        }
    }

    #[test]
    fn export_round_trips() {
        let text = render("mine", "d", vec!["git".into()]);
        let m = parse("mine", &text).unwrap();
        assert_eq!(m.profile.description, "d");
        assert_eq!(m.packages.official, ["git"]);
        assert!(parse("new", &template("new")).is_ok());
    }

    #[test]
    fn shipped_profiles_are_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            Manifest::load(&p).unwrap_or_else(|e| panic!("{e}"));
            n += 1;
        }
        assert!(
            n >= 3,
            "expected core, desktop, developer in {}",
            dir.display()
        );
    }
}
