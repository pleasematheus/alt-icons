//! A Windows executable that permanently changes its own icon.
//!
//! The model is borrowed from iOS alternate icons: a closed set of icons, declared
//! at build time, switched by name at runtime. What is *not* borrowed is where the
//! icon lives. On iOS and Android the thing that changes is the launcher entry; here
//! it is the executable file itself, so the new icon is what Explorer shows for the
//! `.exe`, and it survives a reboot.
//!
//! ```ignore
//! alt_icons::include_icons!();
//!
//! fn main() -> Result<(), alt_icons::Error> {
//!     alt_icons::init()?;
//!     alt_icons::set_icon(AppIcon::Dark)?;
//!     Ok(())
//! }
//! ```
//!
//! Declaring the set lives in `build.rs`, via `alt-icons-build`. That is not an
//! accident of style: a build script runs before macro expansion, so it cannot read
//! a list declared by a macro, and the default icon has to be baked at build time or
//! the binary you ship has no icon at all.
//!
//! # What this costs
//!
//! Every swap rewrites the executable, so its hash changes. Heuristic antivirus and
//! SmartScreen reputation both notice a binary that rewrites itself. The default
//! icon is baked at build time precisely so this only ever happens when your user
//! asks for a different icon, never on first launch of a fresh download.
//!
//! Signing is incompatible: rewriting the PE invalidates an Authenticode signature.

use std::fmt;
use std::path::PathBuf;

#[cfg(windows)]
mod ico;
#[cfg(windows)]
mod resources;
#[cfg(windows)]
mod swap;
#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use swap::{current_icon, init, set_icon};

/// One icon in the set. Implemented by the `AppIcon` enum that `alt-icons-build`
/// generates; there is no reason to implement it by hand.
pub trait Icon {
    /// The variant's name, stored in the binary so [`current_icon`] can report it.
    fn name(&self) -> &'static str;
    /// The raw `.ico` bytes, embedded at build time.
    fn bytes(&self) -> &'static [u8];
}

/// Pulls in the generated `AppIcon` enum. Call once, at the root of your crate.
#[macro_export]
macro_rules! include_icons {
    () => {
        include!(concat!(env!("OUT_DIR"), "/alt_icons.rs"));
    };
}

/// Everything that can go wrong during a swap.
#[derive(Debug)]
pub enum Error {
    /// Another process is mid-swap on this same executable.
    SwapInProgress,
    /// The directory holding the executable cannot be written to.
    ///
    /// The crate deliberately stops here rather than copying itself somewhere
    /// writable or prompting for elevation: a binary that clones itself around the
    /// disk to change a cosmetic detail is exactly the behaviour that gets it
    /// flagged, and a UAC prompt in the middle of an icon change is worse than the
    /// icon not changing.
    PathNotWritable(PathBuf),
    /// The `.ico` could not be parsed. Nothing was written.
    MalformedIcon(&'static str),
    /// A Win32 call failed, named along with its `GetLastError` code.
    Windows(&'static str, u32),
    /// A filesystem operation failed.
    Io(std::io::Error),
    /// The crate only does anything on Windows.
    UnsupportedPlatform,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::SwapInProgress => {
                write!(
                    f,
                    "another process is already changing this executable's icon"
                )
            }
            Error::PathNotWritable(dir) => {
                write!(f, "cannot write to {}", dir.display())
            }
            Error::MalformedIcon(why) => write!(f, "malformed icon: {why}"),
            Error::Windows(call, code) => write!(f, "{call} failed with error {code}"),
            Error::Io(err) => write!(f, "{err}"),
            Error::UnsupportedPlatform => write!(f, "alt-icons only works on Windows"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err)
    }
}

#[cfg(not(windows))]
mod stubs {
    use super::{Error, Icon};

    /// No-op away from Windows, so a cross-platform crate still builds.
    pub fn init() -> Result<(), Error> {
        Ok(())
    }

    pub fn set_icon<I: Icon>(_icon: I) -> Result<(), Error> {
        Err(Error::UnsupportedPlatform)
    }

    pub fn current_icon() -> Result<Option<String>, Error> {
        Ok(None)
    }
}

#[cfg(not(windows))]
pub use stubs::{current_icon, init, set_icon};
