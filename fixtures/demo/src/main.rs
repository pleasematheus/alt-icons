//! Fixture for the integration tests, and the smallest honest example of the API.
//!
//! ```text
//! demo                        prints which icon is currently on the binary
//! demo Dark                   switches to the Dark icon
//! demo Default                switches back
//! demo cycle Dark Default     switches several times within one process
//! demo --keep-old Dark        switches while preserving renamed executables
//! demo hold Dark              switches and waits for a line on stdin
//! ```
//!
//! `cycle` exists for the tests. The first swap of a run has to rename the running
//! image out of the way; every later swap in that same process finds the path free
//! and takes a different code path. Separate invocations can only ever exercise the
//! first branch.

use std::io::{self, Write};
use std::process::ExitCode;

alt_icons::include_icons!();

fn main() -> ExitCode {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run() -> Result<(), alt_icons::Error> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--keep-old") {
        args.remove(0);
        alt_icons::init_with_options(alt_icons::Options { cleanup_old: false })?;
    } else {
        alt_icons::init()?;
    }
    match args.split_first() {
        None => {
            let current = alt_icons::current_icon()?;
            println!("{}", current.as_deref().unwrap_or("Default"));
        }
        Some((first, rest)) if first == "cycle" => {
            for wanted in rest {
                alt_icons::set_icon(lookup(wanted))?;
            }
            let current = alt_icons::current_icon()?;
            println!("{}", current.as_deref().unwrap_or("Default"));
        }
        Some((first, rest)) if first == "hold" => {
            if let Some(wanted) = rest.first() {
                alt_icons::set_icon(lookup(wanted))?;
            }
            let current = alt_icons::current_icon()?;
            println!("{}", current.as_deref().unwrap_or("Default"));
            io::stdout().flush()?;
            // Tests release this process by closing stdin, or kill it to simulate
            // an abrupt exit. Neither requires a fixed delay in the application.
            io::stdin().read_line(&mut String::new())?;
        }
        Some((wanted, _)) => {
            alt_icons::set_icon(lookup(wanted))?;
            println!("{wanted}");
        }
    }
    Ok(())
}

fn lookup(wanted: &str) -> AppIcon {
    AppIcon::ALL
        .iter()
        .find(|icon| alt_icons::Icon::name(*icon) == wanted)
        .copied()
        .unwrap_or_else(|| {
            eprintln!("unknown icon `{wanted}`");
            std::process::exit(2);
        })
}
