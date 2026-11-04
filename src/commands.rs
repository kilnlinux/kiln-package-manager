//! Read-only commands, plus the two that write profile files for the user.

use crate::error::{Result, io_err};
use crate::manifest::{self, Manifest};
use crate::state::{self, State};
use crate::{pacman, ui};
use std::path::{Path, PathBuf};

pub fn version() -> Result<()> {
    println!("kiln {}", env!("CARGO_PKG_VERSION"));
    match pacman::version() {
        Ok(v) => println!("{v}"),
        Err(e) => println!("pacman: {e}"),
    }
    Ok(())
}

pub fn list() -> Result<()> {
    let state = State::load()?;
    if state.profiles.is_empty() {
        println!("no active profiles (see `kiln profile list`)");
        return Ok(());
    }
    for (name, p) in &state.profiles {
        ui::heading(&format!(
            "{} {}",
            ui::accent(name),
            ui::dim(&format!("({} managed)", p.managed.len()))
        ));
        ui::list(&p.managed.iter().cloned().collect::<Vec<_>>());
    }
    Ok(())
}

pub fn search(terms: &[String]) -> Result<()> {
    if !pacman::search(terms)? {
        println!("no packages match");
    }
    Ok(())
}

pub fn profile_list() -> Result<()> {
    let state = State::load()?;
    let found = manifest::discover();
    if found.is_empty() {
        println!("no profiles found");
    }
    for (name, path) in found {
        let mark = if state.profiles.contains_key(&name) {
            ui::good("*")
        } else {
            " ".into()
        };
        let desc = match Manifest::load(&path) {
            Ok(m) => m.profile.description,
            Err(_) => ui::bad("invalid (run `kiln doctor`)"),
        };
        println!(
            "{mark} {:<14} {desc}  {}",
            ui::accent(&name),
            ui::dim(&path.display().to_string())
        );
    }
    println!("{}", ui::dim("* active"));
    Ok(())
}

pub fn profile_show(arg: &str) -> Result<()> {
    let path = manifest::resolve(arg)?;
    let m = Manifest::load(&path)?;
    let state = State::load()?;
    let missing = pacman::unsatisfied(&m.packages.official)?;
    let managed = state.profiles.get(&m.profile.name).map(|p| &p.managed);

    ui::heading(&ui::accent(&m.profile.name));
    if !m.profile.description.is_empty() {
        println!("   {}", m.profile.description);
    }
    println!("   file:   {}", path.display());
    println!(
        "   status: {}",
        if managed.is_some() {
            ui::good("active")
        } else {
            "inactive".into()
        }
    );
    println!("   packages ({}):", m.packages.official.len());
    for p in &m.packages.official {
        let s = if missing.contains(p) {
            ui::warn("missing")
        } else if managed.is_some_and(|m| m.contains(p)) {
            ui::good("installed (kiln)")
        } else {
            ui::good("installed")
        };
        println!("   {p:<24} {s}");
    }
    Ok(())
}

/// Path for a new profile file in the user's directory; refuses to overwrite.
fn new_profile_path(name: &str) -> Result<PathBuf> {
    if !manifest::valid_profile_name(name) {
        return Err(crate::error::Error::Usage(format!(
            "invalid profile name {name:?} (use a-z, 0-9, '-', '_')"
        )));
    }
    let Some(dir) = manifest::user_dir() else {
        return crate::fail!("cannot find home directory (HOME unset)");
    };
    let path = dir.join(format!("{name}.toml"));
    if path.exists() {
        return crate::fail!("{} already exists", path.display());
    }
    std::fs::create_dir_all(&dir).map_err(io_err("creating", &dir))?;
    Ok(path)
}

fn write_new(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text).map_err(io_err("writing", path))?;
    println!("wrote {}", path.display());
    Ok(())
}

pub fn profile_create(name: &str) -> Result<()> {
    let path = new_profile_path(name)?;
    write_new(&path, &manifest::template(name))
}

pub fn export(name: &str, packages: Vec<String>) -> Result<()> {
    let (packages, desc) = if packages.is_empty() {
        (
            pacman::explicit()?,
            "Exported from explicitly installed packages",
        )
    } else {
        (packages, "Exported package set")
    };
    let mut packages = packages;
    packages.sort();
    packages.dedup();
    if let Some(bad) = packages.iter().find(|p| !manifest::valid_package_name(p)) {
        return crate::fail!("invalid package name {bad:?}");
    }
    let path = new_profile_path(name)?;
    write_new(&path, &manifest::render(name, desc, packages))
}

/// Each check prints one line. Returns Err (exit 1) if any check failed.
pub fn doctor() -> Result<()> {
    let mut failed = 0;
    let mut report = |ok: Option<bool>, msg: &str| {
        let tag = match ok {
            Some(true) => ui::good(" ok "),
            None => ui::warn("warn"),
            Some(false) => {
                failed += 1;
                ui::bad("FAIL")
            }
        };
        println!("[{tag}] {msg}");
    };

    match pacman::version() {
        Ok(v) => report(Some(true), &format!("pacman available: {v}")),
        Err(e) => {
            report(Some(false), &e.to_string());
            return crate::fail!("pacman is required");
        }
    }

    if Path::new("/var/lib/pacman/db.lck").exists() {
        report(
            None,
            "pacman database is locked (/var/lib/pacman/db.lck); if no pacman is running, delete it",
        );
    } else {
        report(Some(true), "pacman database not locked");
    }

    match pacman::check_deps() {
        Ok(p) if p.is_empty() => report(
            Some(true),
            "installed packages have all their dependencies (pacman -Dk)",
        ),
        Ok(p) => report(
            Some(false),
            &format!(
                "dependency problems (pacman -Dk):\n       {}",
                p.join("\n       ")
            ),
        ),
        Err(e) => report(Some(false), &e.to_string()),
    }

    for (name, path) in manifest::discover() {
        if let Err(e) = Manifest::load(&path) {
            report(Some(false), &format!("profile {name}: {e}"));
        }
    }

    let state = match State::load() {
        Ok(s) => {
            report(
                Some(true),
                &format!("kiln state readable ({})", state::path().display()),
            );
            s
        }
        Err(e) => {
            report(Some(false), &e.to_string());
            State::default()
        }
    };

    let installed = pacman::installed().unwrap_or_default();
    for (name, ps) in &state.profiles {
        match ps.load_manifest(name) {
            Err(e) => report(Some(false), &format!("active profile {name}: {e}")),
            Ok(m) => match pacman::unsatisfied(&m.packages.official) {
                Ok(miss) if miss.is_empty() => {
                    report(Some(true), &format!("active profile {name} is satisfied"))
                }
                Ok(miss) => report(
                    None,
                    &format!(
                        "active profile {name} is missing: {} (run `sudo kiln apply {name}`)",
                        miss.join(" ")
                    ),
                ),
                Err(e) => report(Some(false), &e.to_string()),
            },
        }
        let gone: Vec<&str> = ps
            .managed
            .iter()
            .filter(|p| !installed.contains(*p))
            .map(String::as_str)
            .collect();
        if !gone.is_empty() {
            report(
                None,
                &format!(
                    "profile {name}: kiln-managed but uninstalled outside kiln: {}",
                    gone.join(" ")
                ),
            );
        }
    }

    if failed > 0 {
        crate::fail!("{failed} check(s) failed")
    } else {
        Ok(())
    }
}
