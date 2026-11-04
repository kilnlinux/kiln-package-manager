//! Pure planning logic: given what pacman reports, decide what to do.
//! No I/O here, so every rule is unit-tested.

use crate::state::State;
use std::collections::HashSet;

#[derive(Debug, Default, PartialEq)]
pub struct ApplyPlan {
    /// Satisfied already (installed, or provided by an installed package).
    pub satisfied: Vec<String>,
    /// Missing and found in the sync databases: pacman will install these.
    pub install: Vec<String>,
    /// Missing and not in any sync database. Apply refuses while any exist.
    pub unknown: Vec<String>,
}

/// `wanted`: the manifest's packages. `unsatisfied`: what `pacman -T` said is
/// missing. `available`: sync-database package names.
pub fn apply_plan(
    wanted: &[String],
    unsatisfied: &[String],
    available: &HashSet<String>,
) -> ApplyPlan {
    let missing: HashSet<&String> = unsatisfied.iter().collect();
    let mut plan = ApplyPlan::default();
    for p in wanted {
        if !missing.contains(p) {
            plan.satisfied.push(p.clone());
        } else if available.contains(p) {
            plan.install.push(p.clone());
        } else {
            plan.unknown.push(p.clone());
        }
    }
    plan
}

#[derive(Debug, Default, PartialEq)]
pub struct RemovePlan {
    /// Kiln installed these for this profile and nothing else claims them.
    pub remove: Vec<String>,
    /// Kiln-managed, but another active profile still wants them.
    pub shared: Vec<String>,
    /// Kiln-managed, but already gone from the system.
    pub gone: Vec<String>,
}

/// Which packages `kiln remove <profile> --purge` may uninstall.
///
/// Only packages Kiln recorded as installed for this profile are candidates.
/// A candidate is kept if another active profile either manages it or lists
/// it in its manifest (`other_wanted`). Manually installed packages are never
/// recorded, so they are never candidates.
pub fn remove_plan(
    profile: &str,
    state: &State,
    other_wanted: &HashSet<String>,
    installed: &HashSet<String>,
) -> RemovePlan {
    let mut plan = RemovePlan::default();
    let Some(mine) = state.profiles.get(profile) else {
        return plan;
    };
    let other_managed: HashSet<&String> = state
        .profiles
        .iter()
        .filter(|(name, _)| name.as_str() != profile)
        .flat_map(|(_, p)| &p.managed)
        .collect();
    for pkg in &mine.managed {
        if other_managed.contains(pkg) || other_wanted.contains(pkg) {
            plan.shared.push(pkg.clone());
        } else if installed.contains(pkg) {
            plan.remove.push(pkg.clone());
        } else {
            plan.gone.push(pkg.clone());
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }
    fn set(xs: &[&str]) -> HashSet<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn apply_sorts_packages_into_buckets() {
        let plan = apply_plan(
            &v(&["git", "gcc", "nope"]),
            &v(&["gcc", "nope"]),
            &set(&["git", "gcc"]),
        );
        assert_eq!(plan.satisfied, v(&["git"]));
        assert_eq!(plan.install, v(&["gcc"]));
        assert_eq!(plan.unknown, v(&["nope"]));
    }

    #[test]
    fn apply_on_satisfied_profile_is_a_no_op() {
        let plan = apply_plan(&v(&["git"]), &[], &set(&[]));
        assert!(plan.install.is_empty() && plan.unknown.is_empty());
    }

    fn state(entries: &[(&str, &[&str])]) -> State {
        let mut s = State::default();
        for (name, pkgs) in entries {
            let e = s.profiles.entry(name.to_string()).or_default();
            e.managed.extend(pkgs.iter().map(|p| p.to_string()));
        }
        s
    }

    #[test]
    fn remove_keeps_packages_other_profiles_need() {
        let s = state(&[("a", &["x", "y", "z", "w"]), ("b", &["y"])]);
        // "z" is listed in b's manifest even though Kiln installed it for a.
        let plan = remove_plan("a", &s, &set(&["z"]), &set(&["x", "y", "z"]));
        assert_eq!(plan.remove, v(&["x"]));
        assert_eq!(plan.shared, v(&["y", "z"]));
        assert_eq!(plan.gone, v(&["w"]));
    }

    #[test]
    fn remove_never_touches_unmanaged_packages() {
        // "vim" is installed but Kiln did not install it: never a candidate.
        let s = state(&[("a", &["x"])]);
        let plan = remove_plan("a", &s, &set(&[]), &set(&["x", "vim"]));
        assert_eq!(plan.remove, v(&["x"]));
    }

    #[test]
    fn remove_of_inactive_profile_is_empty() {
        assert_eq!(
            remove_plan("a", &State::default(), &set(&[]), &set(&[])),
            RemovePlan::default()
        );
    }
}
