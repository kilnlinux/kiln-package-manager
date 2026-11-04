//! End-to-end tests of the kiln binary against a fake `pacman` placed first
//! on PATH. The fake answers only the queries kiln makes, from two env vars,
//! so these tests are deterministic and never touch the real system.
//! Commands that need root are tested up to the root check, and through
//! their --dry-run plans.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FAKE_PACMAN: &str = r#"#!/bin/sh
# FAKE_INSTALLED / FAKE_AVAILABLE: space-separated package names.
case "$1" in
  -T) shift; [ "$1" = "--" ] && shift
      miss=0
      for p in "$@"; do
        case " $FAKE_INSTALLED " in *" $p "*) ;; *) echo "$p"; miss=1 ;; esac
      done
      [ $miss = 1 ] && exit 127; exit 0 ;;
  -Qq|-Qqe) printf '%s\n' $FAKE_INSTALLED ;;
  -Slq) printf '%s\n' $FAKE_AVAILABLE ;;
  -Dk) echo "No database errors have been found!" ;;
  -R) printf '%s\n' "$*" >> "$FAKE_TRANSACTIONS" ;;
  --version) echo " .--.   Pacman v9.9.9 - libalpm v1.0.0" ;;
  *) echo "fake pacman: unexpected $*" >&2; exit 99 ;;
esac
"#;

struct Env {
    dir: PathBuf,
}

impl Env {
    fn new(test: &str) -> Env {
        let dir = std::env::temp_dir().join(format!("kiln-it-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::create_dir_all(dir.join("config/kiln/profiles")).unwrap();
        let fake = dir.join("bin/pacman");
        std::fs::write(&fake, FAKE_PACMAN).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        Env { dir }
    }

    fn profile(&self, name: &str, pkgs: &[&str]) {
        let list: Vec<String> = pkgs.iter().map(|p| format!("\"{p}\"")).collect();
        let text = format!(
            "[profile]\nname = \"{name}\"\n[packages]\nofficial = [{}]\n",
            list.join(", ")
        );
        std::fs::write(
            self.dir.join(format!("config/kiln/profiles/{name}.toml")),
            text,
        )
        .unwrap();
    }

    fn state(&self, text: &str) {
        std::fs::write(self.dir.join("state.toml"), text).unwrap();
    }

    fn kiln(&self, args: &[&str], installed: &str, available: &str) -> Output {
        let path = format!(
            "{}:{}",
            self.dir.join("bin").display(),
            std::env::var("PATH").unwrap()
        );
        Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(args)
            .env("PATH", path)
            .env("XDG_CONFIG_HOME", self.dir.join("config"))
            .env("KILN_STATE_FILE", self.dir.join("state.toml"))
            .env("FAKE_INSTALLED", installed)
            .env("FAKE_AVAILABLE", available)
            .env("FAKE_TRANSACTIONS", self.dir.join("transactions"))
            .env("NO_COLOR", "1")
            .env_remove("SUDO_USER")
            .output()
            .unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn running_as_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(2) == Some("0"))
}

#[test]
fn help_version_and_usage_errors() {
    let e = Env::new("basic");
    assert!(stdout(&e.kiln(&["help"], "", "")).contains("kiln apply"));
    let v = e.kiln(&["version"], "", "");
    assert!(stdout(&v).contains("kiln 0.") && stdout(&v).contains("Pacman v9.9.9"));
    let bad = e.kiln(&["frobnicate"], "", "");
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn plan_buckets_and_unknown_packages() {
    let e = Env::new("plan");
    e.profile("dev", &["git", "gcc"]);
    let o = e.kiln(&["plan", "dev"], "git", "git gcc");
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("1 of 2 packages installed"));
    assert!(stdout(&o).contains("to install (1)") && stdout(&o).contains("gcc"));
    assert!(stdout(&o).contains("dry run"));

    e.profile("typo", &["gti"]);
    let o = e.kiln(&["plan", "typo"], "git", "git");
    assert_eq!(o.status.code(), Some(1));
    assert!(stdout(&o).contains("not found in any sync repository"));
}

#[test]
fn apply_satisfied_and_active_is_a_no_op_without_root() {
    let e = Env::new("idem");
    e.profile("dev", &["git"]);
    e.state("[profiles.dev]\nmanaged = []\n");
    let o = e.kiln(&["apply", "dev"], "git", "git");
    assert!(o.status.success(), "{}", stderr(&o));
    if !running_as_root() {
        assert!(stdout(&o).contains("satisfied, nothing to do"));
    }
}

#[test]
fn apply_needs_root_and_records_nothing() {
    if running_as_root() {
        return;
    }
    let e = Env::new("root");
    e.profile("dev", &["gcc"]);
    let o = e.kiln(&["apply", "dev"], "", "gcc");
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("run it with sudo"));
    assert!(!e.dir.join("state.toml").exists());
}

#[test]
fn purge_plan_keeps_shared_and_unmanaged_packages() {
    let e = Env::new("purge");
    e.profile("a", &["x", "y", "vim"]);
    e.profile("b", &["y"]);
    // Kiln installed x and y for a; vim was already there, so not managed.
    e.state("[profiles.a]\nmanaged = [\"x\", \"y\"]\n[profiles.b]\nmanaged = []\n");
    let o = e.kiln(&["remove", "a", "--purge", "--dry-run"], "x y vim", "");
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    let uninstall = out.split("to uninstall").nth(1).expect("uninstall section");
    assert!(uninstall.contains('x') && !uninstall.contains("vim") && !uninstall.contains(" y"));
    assert!(out.contains("kept, another active profile needs them (1)"));
}

#[test]
fn export_and_profile_list() {
    let e = Env::new("export");
    let o = e.kiln(&["export", "mine"], "bash git", "");
    assert!(o.status.success(), "{}", stderr(&o));
    let text = std::fs::read_to_string(e.dir.join("config/kiln/profiles/mine.toml")).unwrap();
    assert!(text.contains("\"bash\"") && text.contains("\"git\""));
    // Refuses to overwrite.
    assert_eq!(
        e.kiln(&["export", "mine", "git"], "", "").status.code(),
        Some(1)
    );
    assert!(stdout(&e.kiln(&["profile", "list"], "", "")).contains("mine"));
}

#[test]
fn doctor_warns_about_missing_profile_packages() {
    let e = Env::new("doctor");
    e.profile("dev", &["git", "gcc"]);
    e.state("[profiles.dev]\nmanaged = [\"gcc\"]\n");
    let o = e.kiln(&["doctor"], "git", "");
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("active profile dev is missing: gcc"));
    assert!(out.contains("uninstalled outside kiln: gcc"));
}

#[test]
fn shipped_profiles_plan_cleanly() {
    let e = Env::new("shipped");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("profiles");
    let mut profiles: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    profiles.sort();
    // Pretend every quoted string in the profiles is an available package.
    let available: Vec<String> = profiles
        .iter()
        .flat_map(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            text.split('"')
                .skip(1)
                .step_by(2)
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect();
    let available = available.join(" ");
    assert!(
        profiles.len() >= 10,
        "expected all shipped profiles in {}",
        dir.display()
    );
    for p in &profiles {
        let o = e.kiln(&["plan", p.to_str().unwrap()], "", &available);
        assert!(
            o.status.success(),
            "{}: {}{}",
            p.display(),
            stdout(&o),
            stderr(&o)
        );
    }
}

#[test]
fn custom_profile_paths_work_for_purge_and_doctor() {
    let e = Env::new("custom-path");
    e.profile("a", &["parent"]);
    let custom = e.dir.join("custom");
    std::fs::create_dir_all(&custom).unwrap();
    let path = custom.join("b.toml");
    std::fs::write(
        &path,
        "[profile]\nname='b'\n[packages]\nofficial=['shared']\n",
    )
    .unwrap();
    e.state(&format!(
        "[profiles.a]\nmanaged=['parent', 'shared']\n[profiles.b]\nmanaged=[]\nmanifest={:?}\n",
        path.to_str().unwrap()
    ));
    let o = e.kiln(
        &["remove", "a", "--purge", "--dry-run"],
        "parent shared",
        "",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("kept, another active profile needs them (1)"));
    let o = e.kiln(&["doctor"], "parent shared", "");
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("active profile b is satisfied"));

    std::fs::write(
        &path,
        "[profile]\nname='b'\n[packages]\nofficial=['manual']\n",
    )
    .unwrap();
    let o = e.kiln(
        &["remove", "a", "--purge", "--dry-run"],
        "parent shared manual",
        "",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!stdout(&o).contains("kept, another active profile"));

    std::fs::remove_file(&path).unwrap();
    let o = e.kiln(
        &["remove", "a", "--purge", "--dry-run"],
        "parent shared",
        "",
    );
    assert!(!o.status.success());
    assert!(stderr(&o).contains("cannot read active profile"));
}

#[test]
fn apply_saves_absolute_custom_path_when_root() {
    if !running_as_root() {
        return;
    }
    let e = Env::new("save-custom-path");
    let custom = e.dir.join("custom");
    std::fs::create_dir_all(&custom).unwrap();
    let path = custom.join("dev.toml");
    std::fs::write(
        &path,
        "[profile]\nname='dev'\n[packages]\nofficial=['git']\n",
    )
    .unwrap();
    let o = e.kiln(&["apply", path.to_str().unwrap(), "--yes"], "git", "");
    assert!(o.status.success(), "{}", stderr(&o));
    let state: toml::Value =
        toml::from_str(&std::fs::read_to_string(e.dir.join("state.toml")).unwrap()).unwrap();
    assert_eq!(
        state["profiles"]["dev"]["manifest"].as_str(),
        path.canonicalize().unwrap().to_str()
    );
    assert_eq!(
        state["profiles"]["dev"]["managed"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let o = e.kiln(&["apply", path.to_str().unwrap(), "--yes"], "git", "");
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("satisfied, nothing to do"));
}

#[test]
fn purge_transfers_shared_packages_then_removes_them_when_root() {
    if !running_as_root() {
        return;
    }
    let e = Env::new("transfer-purge");
    e.profile("a", &["parent", "shared"]);
    e.profile("b", &["shared", "manual"]);
    e.state("[profiles.a]\nmanaged=['parent','shared']\n[profiles.b]\nmanaged=[]\n");
    let o = e.kiln(
        &["remove", "a", "--purge", "--yes"],
        "parent shared manual",
        "",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let state: toml::Value =
        toml::from_str(&std::fs::read_to_string(e.dir.join("state.toml")).unwrap()).unwrap();
    assert!(state["profiles"].get("a").is_none());
    assert_eq!(
        state["profiles"]["b"]["managed"].as_array().unwrap(),
        &[toml::Value::String("shared".into())]
    );
    let o = e.kiln(&["remove", "b", "--purge", "--yes"], "shared manual", "");
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(
        std::fs::read_to_string(e.dir.join("transactions")).unwrap(),
        "-R --noconfirm -- parent\n-R --noconfirm -- shared\n"
    );
}
