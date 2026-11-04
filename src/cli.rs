//! Command-line parsing. Small enough that a hand-written parser is clearer
//! than pulling in an argument-parsing framework.

use crate::error::{Error, Result};

#[derive(Debug, PartialEq)]
pub enum Cmd {
    Help,
    Version,
    List,
    Search(Vec<String>),
    ProfileList,
    ProfileShow(String),
    ProfileCreate(String),
    Plan(String),
    Apply {
        profile: String,
        dry_run: bool,
        yes: bool,
    },
    Remove {
        profile: String,
        purge: bool,
        dry_run: bool,
        yes: bool,
    },
    Doctor,
    Export {
        name: String,
        packages: Vec<String>,
    },
}

pub const HELP: &str = "\
kiln - declarative package profiles for Kiln Linux

pacman installs packages. kiln keeps named sets of them (profiles) in
readable TOML files and brings the system in line with them.

Usage: kiln <command> [options]

Commands:
  list                      Active profiles and the packages kiln manages
  search <query>...         Search the official repositories (pacman -Ss)
  profile list              All profiles kiln can find
  profile show <profile>    A profile's packages and whether they're installed
  profile create <name>     New empty profile in ~/.config/kiln/profiles
  plan <profile>            What `apply` would change; changes nothing
  apply <profile>           Install what the profile is missing, mark it active
  remove <profile>          Mark a profile inactive (keeps its packages)
  doctor                    Check pacman, dependencies and profiles
  export <name> [pkg...]    Write a profile from the given packages, or from
                            all explicitly installed ones (pacman -Qqe)
  version                   Print versions
  help                      This text

Options:
  -n, --dry-run   apply, remove: show the plan only
  -y, --yes       apply, remove: don't ask for confirmation (pacman --noconfirm)
      --purge     remove: also uninstall packages kiln installed for this
                  profile that no other active profile needs

<profile> is a name (searched in ~/.config/kiln/profiles, /etc/kiln/profiles,
/usr/share/kiln/profiles) or a path to a .toml file.

apply and remove need root: sudo kiln apply <profile>
";

/// Split arguments into positionals and the flags we recognise.
struct Args {
    pos: Vec<String>,
    dry_run: bool,
    yes: bool,
    purge: bool,
}

fn split(args: &[String]) -> Result<Args> {
    let mut a = Args {
        pos: Vec::new(),
        dry_run: false,
        yes: false,
        purge: false,
    };
    let mut only_pos = false;
    for arg in args {
        match arg.as_str() {
            _ if only_pos => a.pos.push(arg.clone()),
            "--" => only_pos = true,
            "-n" | "--dry-run" => a.dry_run = true,
            "-y" | "--yes" => a.yes = true,
            "--purge" => a.purge = true,
            s if s.starts_with('-') && s.len() > 1 => {
                return Err(Error::Usage(format!(
                    "unknown option {s:?} (see `kiln help`)"
                )));
            }
            _ => a.pos.push(arg.clone()),
        }
    }
    Ok(a)
}

fn one(cmd: &str, pos: &[String]) -> Result<String> {
    match pos {
        [x] => Ok(x.clone()),
        _ => Err(Error::Usage(format!(
            "`kiln {cmd}` takes exactly one argument"
        ))),
    }
}

pub fn parse(args: &[String]) -> Result<Cmd> {
    let Some((first, rest)) = args.split_first() else {
        return Ok(Cmd::Help);
    };
    let a = split(rest)?;
    let cmd = first.as_str();

    // Reject flags on commands that don't take them, instead of ignoring them.
    let flags_ok = match cmd {
        "apply" => !a.purge,
        "remove" => true,
        _ => !(a.dry_run || a.yes || a.purge),
    };
    if !flags_ok {
        return Err(Error::Usage(format!("option not valid for `kiln {cmd}`")));
    }
    let none = |c: Cmd| {
        if a.pos.is_empty() {
            Ok(c)
        } else {
            Err(Error::Usage(format!("`kiln {cmd}` takes no arguments")))
        }
    };

    match cmd {
        "help" | "-h" | "--help" => Ok(Cmd::Help),
        "version" | "-V" | "--version" => none(Cmd::Version),
        "list" => none(Cmd::List),
        "doctor" => none(Cmd::Doctor),
        "search" if a.pos.is_empty() => Err(Error::Usage("`kiln search` needs a query".into())),
        "search" => Ok(Cmd::Search(a.pos)),
        "plan" => Ok(Cmd::Plan(one(cmd, &a.pos)?)),
        "apply" => Ok(Cmd::Apply {
            profile: one(cmd, &a.pos)?,
            dry_run: a.dry_run,
            yes: a.yes,
        }),
        "remove" => Ok(Cmd::Remove {
            profile: one(cmd, &a.pos)?,
            purge: a.purge,
            dry_run: a.dry_run,
            yes: a.yes,
        }),
        "export" => match a.pos.split_first() {
            Some((name, pkgs)) => Ok(Cmd::Export {
                name: name.clone(),
                packages: pkgs.to_vec(),
            }),
            None => Err(Error::Usage("`kiln export` needs a profile name".into())),
        },
        "profile" => match a.pos.split_first() {
            Some((sub, rest)) => match sub.as_str() {
                "list" if rest.is_empty() => Ok(Cmd::ProfileList),
                "show" => Ok(Cmd::ProfileShow(one("profile show", rest)?)),
                "create" => Ok(Cmd::ProfileCreate(one("profile create", rest)?)),
                _ => Err(Error::Usage(
                    "unknown `kiln profile` usage (see `kiln help`)".into(),
                )),
            },
            None => Err(Error::Usage(
                "`kiln profile` needs list, show or create".into(),
            )),
        },
        _ => Err(Error::Usage(format!(
            "unknown command {cmd:?} (see `kiln help`)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Cmd> {
        parse(&s.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    #[test]
    fn parses_commands() {
        assert_eq!(p("").unwrap(), Cmd::Help);
        assert_eq!(p("profile list").unwrap(), Cmd::ProfileList);
        assert_eq!(
            p("apply dev -n").unwrap(),
            Cmd::Apply {
                profile: "dev".into(),
                dry_run: true,
                yes: false
            }
        );
        assert_eq!(
            p("remove dev --purge --yes").unwrap(),
            Cmd::Remove {
                profile: "dev".into(),
                purge: true,
                dry_run: false,
                yes: true
            }
        );
        assert_eq!(
            p("export mine git gcc").unwrap(),
            Cmd::Export {
                name: "mine".into(),
                packages: vec!["git".into(), "gcc".into()]
            }
        );
        assert_eq!(
            p("search -- -weird").unwrap(),
            Cmd::Search(vec!["-weird".into()])
        );
    }

    #[test]
    fn rejects_bad_usage() {
        for bad in [
            "bogus",
            "apply",
            "apply a b",
            "apply a --purge",
            "list --yes",
            "plan a --bogus",
            "profile",
            "profile show",
            "search",
        ] {
            assert!(matches!(p(bad), Err(Error::Usage(_))), "{bad}");
        }
    }
}
