//! `kiln apply`, `kiln sync` and `kiln remove`: the commands that change the system.
//!
//! Order matters for safety: plan from pacman's live view, let pacman run its
//! own transaction (with its own lock and confirmation), and only record
//! state after pacman reports success. If pacman fails or the user says no,
//! Kiln's state is untouched. There is no rollback: pacman transactions are
//! all-or-nothing on their own, and Kiln never claims more than that.

use crate::error::Result;
use crate::manifest::{self, Manifest};
use crate::plan;
use crate::state::State;
use crate::{pacman, ui};
use std::collections::{BTreeMap, HashSet};

pub fn apply(arg: &str, purge: bool, dry_run: bool, yes: bool) -> Result<()> {
    let path = manifest::resolve(arg)?;
    let path = std::fs::canonicalize(&path).map_err(crate::error::io_err("resolving", &path))?;
    let m = Manifest::load(&path)?;
    let name = &m.profile.name;
    let wanted = &m.packages.official;

    let unsatisfied = pacman::unsatisfied(wanted)?;
    // Only fetch the sync package list when something is missing.
    let available = if unsatisfied.is_empty() {
        HashSet::new()
    } else {
        pacman::available()?
    };
    let p = plan::apply_plan(wanted, &unsatisfied, &available);
    let initial_state = State::load()?;
    let active = initial_state.profiles.get(name);
    // Recorded for this profile, but no longer in its manifest.
    let dropped: Vec<String> = active
        .map(|a| {
            a.managed
                .iter()
                .filter(|p| !wanted.contains(p))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let prune = if purge && !dropped.is_empty() {
        let (other_wanted, other_profiles) = others(&initial_state, name)?;
        let installed = pacman::installed()?;
        let r = plan::remove_plan(name, &dropped, &initial_state, &other_wanted, &installed);
        Some((r, other_profiles))
    } else {
        None
    };

    ui::heading(&format!(
        "profile {} ({})",
        ui::accent(name),
        path.display()
    ));
    println!(
        "   {} of {} packages installed",
        p.satisfied.len(),
        wanted.len()
    );
    if !p.unknown.is_empty() {
        println!("{}", ui::bad("   not found in any sync repository:"));
        ui::list(&p.unknown);
        return crate::fail!(
            "profile {name:?} lists {} unknown package(s); fix the manifest or run `sudo pacman -Syu` to refresh",
            p.unknown.len()
        );
    }
    match &prune {
        Some((r, _)) => {
            if !r.shared.is_empty() {
                println!(
                    "   no longer listed; kept, another active profile needs them ({}):",
                    r.shared.len()
                );
                ui::list(&r.shared);
            }
            if !r.gone.is_empty() {
                println!(
                    "   no longer listed; already uninstalled ({}):",
                    r.gone.len()
                );
                ui::list(&r.gone);
            }
            if !r.remove.is_empty() {
                println!("   no longer listed; to uninstall ({}):", r.remove.len());
                ui::list(&r.remove);
            }
        }
        None if !dropped.is_empty() => {
            println!(
                "   kiln installed these, but the profile no longer lists them ({}):",
                dropped.len()
            );
            ui::list(&dropped);
            println!(
                "{}",
                ui::dim("   (use --purge to uninstall the ones no other profile needs)")
            );
        }
        None => {}
    }
    if p.install.is_empty() && prune.is_none() {
        if active.is_some_and(|p| p.manifest.as_ref() == Some(&path) || !pacman::is_root()) {
            println!("{}", ui::good("   satisfied, nothing to do"));
            return Ok(());
        }
        println!("   satisfied; will mark the profile active");
    } else if !p.install.is_empty() {
        println!("   to install ({}):", p.install.len());
        ui::list(&p.install);
    }
    if dry_run {
        println!("{}", ui::dim("   dry run: no changes made"));
        return Ok(());
    }
    pacman::require_root("kiln apply")?;

    let mut newly = Vec::new();
    if !p.install.is_empty() {
        if !pacman::install(&p.install, yes)? {
            return crate::fail!("pacman did not complete; kiln recorded no changes");
        }
        // Record only what pacman actually made satisfied.
        let still = pacman::unsatisfied(&p.install)?;
        newly = p
            .install
            .into_iter()
            .filter(|x| !still.contains(x))
            .collect();
    }

    // Reload: another kiln may have saved while pacman ran.
    // shortcut: no lock around state, so two kiln runs saving at the same instant can lose one update; add flock() if that ever matters.
    let mut state = State::load()?;
    let profile = state.profiles.entry(name.clone()).or_default();
    profile.manifest = Some(path);
    profile.managed.extend(newly);
    state.save()?;

    if let Some((r, other_profiles)) = prune {
        if !r.remove.is_empty() && !pacman::remove(&r.remove, yes)? {
            return crate::fail!(
                "pacman did not complete; kiln still records the packages {name} no longer lists"
            );
        }
        let mut state = State::load()?;
        let released: Vec<String> = [r.remove, r.gone, r.shared].concat();
        state.release(name, &released, &other_profiles);
        state.save()?;
    }
    println!(
        "{}",
        ui::good(&format!("   profile {name} is active and satisfied"))
    );
    Ok(())
}

pub fn remove(arg: &str, purge: bool, dry_run: bool, yes: bool) -> Result<()> {
    // Accept a path too, but state is keyed by name.
    let name = if arg.contains('/') || arg.ends_with(".toml") {
        Manifest::load(std::path::Path::new(arg))?.profile.name
    } else {
        arg.to_string()
    };
    let state = State::load()?;
    let Some(mine) = state.profiles.get(&name) else {
        return crate::fail!("profile {name:?} is not active (see `kiln list`)");
    };
    ui::heading(&format!("remove profile {}", ui::accent(&name)));

    if !purge {
        println!(
            "   will mark inactive; its {} kiln-installed package(s) stay installed",
            mine.managed.len()
        );
        println!(
            "{}",
            ui::dim("   (use --purge to uninstall the ones no other profile needs)")
        );
        if dry_run {
            println!("{}", ui::dim("   dry run: no changes made"));
            return Ok(());
        }
        pacman::require_root("kiln remove")?;
        return deactivate(&name, &BTreeMap::new());
    }

    let (other_wanted, other_profiles) = others(&state, &name)?;
    let p = plan::remove_plan(
        &name,
        &mine.managed,
        &state,
        &other_wanted,
        &pacman::installed()?,
    );

    if !p.shared.is_empty() {
        println!(
            "   kept, another active profile needs them ({}):",
            p.shared.len()
        );
        ui::list(&p.shared);
    }
    if !p.gone.is_empty() {
        println!("   already uninstalled ({}):", p.gone.len());
        ui::list(&p.gone);
    }
    if p.remove.is_empty() {
        println!("   nothing to uninstall");
    } else {
        println!("   to uninstall ({}):", p.remove.len());
        ui::list(&p.remove);
    }
    if dry_run {
        println!("{}", ui::dim("   dry run: no changes made"));
        return Ok(());
    }
    pacman::require_root("kiln remove")?;
    if !p.remove.is_empty() && !pacman::remove(&p.remove, yes)? {
        return crate::fail!("pacman did not complete; profile {name} is still active");
    }
    deactivate(&name, &other_profiles)
}

/// `kiln sync`: apply every active profile from its recorded manifest.
pub fn sync(purge: bool, dry_run: bool, yes: bool) -> Result<()> {
    let state = State::load()?;
    if state.profiles.is_empty() {
        println!("no active profiles (see `kiln profile list`)");
        return Ok(());
    }
    for (name, ps) in &state.profiles {
        ps.load_manifest(name)?; // refuses a file now naming another profile
        let path = match &ps.manifest {
            Some(path) => path.clone(),
            None => manifest::resolve(name)?,
        };
        apply(&path.to_string_lossy(), purge, dry_run, yes)?;
    }
    Ok(())
}

/// Everything the other active profiles want right now, merged and per
/// profile. If one of their manifests can't be read we can't prove a package
/// is unneeded: stop.
type Wanted = (HashSet<String>, BTreeMap<String, Vec<String>>);
fn others(state: &State, name: &str) -> Result<Wanted> {
    let mut other_wanted = HashSet::new();
    let mut other_profiles = BTreeMap::new();
    for (other, profile) in state.profiles.iter().filter(|(n, _)| *n != name) {
        let m = profile.load_manifest(other).map_err(|e| {
            crate::error::Error::Failed(format!(
                "cannot read active profile {other:?}, so cannot tell what it needs: {e}"
            ))
        })?;
        other_wanted.extend(m.packages.official.iter().cloned());
        other_profiles.insert(other.clone(), m.packages.official);
    }
    Ok((other_wanted, other_profiles))
}

fn deactivate(name: &str, other_wanted: &BTreeMap<String, Vec<String>>) -> Result<()> {
    let mut state = State::load()?;
    state.deactivate(name, other_wanted);
    state.save()?;
    println!("{}", ui::good(&format!("   profile {name} is inactive")));
    Ok(())
}
