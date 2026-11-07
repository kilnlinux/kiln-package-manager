//! kiln: declarative package profiles on top of pacman.
//!
//! Module map:
//!   cli       argument parsing          manifest  TOML profiles and lookup
//!   pacman    all calls to pacman       plan      pure planning rules
//!   apply     apply / sync / remove           state     /var/lib/kiln/state.toml
//!   commands  the read-only commands    ui, error output and errors

mod apply;
mod cli;
mod commands;
mod error;
mod manifest;
mod pacman;
mod plan;
mod state;
mod ui;

use cli::Cmd;

fn run(cmd: Cmd) -> error::Result<()> {
    match cmd {
        Cmd::Help => {
            print!("{}", cli::HELP);
            Ok(())
        }
        Cmd::Version => commands::version(),
        Cmd::List => commands::list(),
        Cmd::Search(q) => commands::search(&q),
        Cmd::ProfileList => commands::profile_list(),
        Cmd::ProfileShow(p) => commands::profile_show(&p),
        Cmd::ProfileCreate(n) => commands::profile_create(&n),
        Cmd::Plan(p) => apply::apply(&p, false, true, false),
        Cmd::Apply {
            profile,
            purge,
            dry_run,
            yes,
        } => apply::apply(&profile, purge, dry_run, yes),
        Cmd::Sync {
            purge,
            dry_run,
            yes,
        } => apply::sync(purge, dry_run, yes),
        Cmd::Remove {
            profile,
            purge,
            dry_run,
            yes,
        } => apply::remove(&profile, purge, dry_run, yes),
        Cmd::Doctor => commands::doctor(),
        Cmd::Export { name, packages } => commands::export(&name, packages),
    }
}

// Rust ignores SIGPIPE, so `kiln search x | head` would panic on a closed
// pipe. Restore the default (die quietly), like every other CLI tool.
// Declared directly to avoid a libc dependency; glibc is linked anyway.
unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}
const SIGPIPE: i32 = 13;
const SIG_DFL: usize = 0;

fn main() {
    unsafe { signal(SIGPIPE, SIG_DFL) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = cli::parse(&args).and_then(run) {
        eprintln!("kiln: error: {e}");
        std::process::exit(e.exit_code());
    }
}
