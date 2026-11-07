//! Kiln's own state: which profiles are active, and which packages Kiln
//! itself installed for each. Nothing about versions or files; pacman owns
//! that. A package that was already installed before `kiln apply` is never
//! recorded, so `kiln remove --purge` can never take it away.
//!
//! Stored as TOML at /var/lib/kiln/state.toml (override: KILN_STATE_FILE).

use crate::error::{Error, Result, io_err};
use crate::manifest::{self, Manifest};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct State {
    /// Active profiles, by name.
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileState>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct ProfileState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<PathBuf>,
    /// Packages Kiln installed because this profile asked for them.
    #[serde(default)]
    pub managed: BTreeSet<String>,
}

impl ProfileState {
    pub fn load_manifest(&self, name: &str) -> Result<Manifest> {
        let path = match &self.manifest {
            Some(path) => path.clone(),
            None => manifest::resolve(name)?,
        };
        let m = Manifest::load(&path)?;
        if m.profile.name != name {
            return crate::fail!(
                "active profile {name:?} has been replaced by {:?}",
                m.profile.name
            );
        }
        Ok(m)
    }
}

pub fn path() -> PathBuf {
    std::env::var_os("KILN_STATE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/kiln/state.toml"))
}

impl State {
    pub fn deactivate(&mut self, name: &str, other_wanted: &BTreeMap<String, Vec<String>>) {
        if let Some(removed) = self.profiles.remove(name) {
            let pkgs: Vec<String> = removed.managed.into_iter().collect();
            self.release(name, &pkgs, other_wanted);
        }
    }

    /// Stop recording `pkgs` for `name`; other active profiles that list one
    /// take it over, so a later purge of theirs can still remove it.
    pub fn release(
        &mut self,
        name: &str,
        pkgs: &[String],
        other_wanted: &BTreeMap<String, Vec<String>>,
    ) {
        if let Some(profile) = self.profiles.get_mut(name) {
            profile.managed.retain(|p| !pkgs.contains(p));
        }
        for (other, wanted) in other_wanted.iter().filter(|(o, _)| *o != name) {
            if let Some(profile) = self.profiles.get_mut(other) {
                profile
                    .managed
                    .extend(pkgs.iter().filter(|p| wanted.contains(p)).cloned());
            }
        }
    }

    /// Missing file means a fresh system with nothing active.
    pub fn load() -> Result<State> {
        let p = path();
        match std::fs::read_to_string(&p) {
            Ok(text) => toml::from_str(&text).map_err(|e| {
                Error::Failed(format!("{}: corrupt state: {}", p.display(), e.message()))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(io_err("reading", &p)(e)),
        }
    }

    /// Write to a temp file in the same directory, fsync, then rename over the
    /// old file. rename() is atomic, so a crash leaves the old or the new
    /// state, never half of one.
    pub fn save(&self) -> Result<()> {
        use std::io::Write;
        let p = path();
        let dir = p.parent().unwrap_or(std::path::Path::new("."));
        std::fs::create_dir_all(dir).map_err(io_err("creating", dir))?;
        let tmp = p.with_extension("toml.tmp");
        let text = format!(
            "# Managed by kiln. Lists active profiles and the packages kiln installed for them.\n{}",
            toml::to_string(self).expect("state serializes")
        );
        let mut f = std::fs::File::create(&tmp).map_err(io_err("writing", &tmp))?;
        f.write_all(text.as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(io_err("writing", &tmp))?;
        std::fs::rename(&tmp, &p).map_err(io_err("replacing", &p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_ownership_survives_until_the_last_profile_is_removed() {
        use std::collections::HashSet;
        let mut state = State::default();
        state
            .profiles
            .entry("a".into())
            .or_default()
            .managed
            .insert("shared".into());
        state.profiles.entry("b".into()).or_default();
        state.profiles.entry("c".into()).or_default();
        let wanted = BTreeMap::from([
            ("b".into(), vec!["shared".into(), "manual".into()]),
            ("c".into(), vec!["shared".into()]),
        ]);
        state.deactivate("a", &wanted);
        assert!(!state.profiles.contains_key("a"));
        assert_eq!(
            state.profiles["b"].managed,
            BTreeSet::from(["shared".into()])
        );
        assert_eq!(
            state.profiles["c"].managed,
            BTreeSet::from(["shared".into()])
        );
        state.deactivate("b", &BTreeMap::from([("c".into(), vec!["shared".into()])]));
        let plan = crate::plan::remove_plan(
            "c",
            &state.profiles["c"].managed,
            &state,
            &HashSet::new(),
            &HashSet::from(["shared".into(), "manual".into()]),
        );
        assert_eq!(plan.remove, ["shared"]);
    }

    #[test]
    fn old_state_files_need_no_migration() {
        let state: State = toml::from_str("[profiles.dev]\nmanaged = ['git']\n").unwrap();
        assert!(state.profiles["dev"].manifest.is_none());
        assert!(state.profiles["dev"].managed.contains("git"));
    }

    // Single test touching the env var, so parallel tests cannot race on it.
    #[test]
    fn round_trip_and_missing_file() {
        let dir = std::env::temp_dir().join(format!("kiln-state-test-{}", std::process::id()));
        let file = dir.join("sub/state.toml");
        unsafe { std::env::set_var("KILN_STATE_FILE", &file) };

        assert_eq!(State::load().unwrap(), State::default());

        let mut s = State::default();
        s.profiles
            .entry("dev".into())
            .or_default()
            .managed
            .insert("git".into());
        s.profiles.get_mut("dev").unwrap().manifest = Some(dir.join("custom/dev.toml"));
        s.save().unwrap();
        assert_eq!(State::load().unwrap(), s);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
