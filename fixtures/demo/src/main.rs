//! Fixture for the integration tests, and the smallest honest example of the API.
//!
//! ```text
//! demo                        prints which icon is currently on the binary
//! demo Dark                   switches to the Dark icon
//! demo Default                switches back
//! demo cycle Dark Default     switches several times within one process
//! ```
//!
//! `cycle` exists for the tests. The first swap of a run has to rename the running
//! image out of the way; every later swap in that same process finds the path free
//! and takes a different code path. Separate invocations can only ever exercise the
//! first branch.

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
    // Clears the renamed executable left by a previous run, and repairs a swap that
    // was interrupted. Cheap, and the only place it can happen.
    alt_icons::init()?;

    let args: Vec<String> = std::env::args().skip(1).collect();
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
