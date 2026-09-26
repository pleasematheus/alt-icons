//! The swap itself: turn a `.ico` into resources, put them in a copy of the running
//! executable, and move that copy into place.
//!
//! The order matters, and the reasons are measured rather than assumed — see
//! `DESIGN.md`. A running executable cannot be opened for writing, but it *can* be
//! renamed, and a new file can then take its path while the old process keeps
//! running from the renamed one.

use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Mutex, OnceLock,
};

use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;

use crate::ico;
use crate::resources::{self, Update, FIRST_ICON_ID};
use crate::win::{self, SwapLock};
use crate::{Error, Icon};

/// Cleans up after previous swaps, and repairs one that was interrupted.
///
/// Call this once, early in `main`. Every swap leaves behind the renamed previous
/// executable, which stays locked until the process that was running from it exits —
/// so it can only be deleted on a later run. This is that later run.
pub fn init() -> Result<(), Error> {
    let exe = win::current_exe()?;
    let (dir, stem) = location(&exe)?;

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // A directory we cannot even list is not a reason to stop the program;
        // there is simply nothing to clean.
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_leftover(name, &stem) {
            continue;
        }
        // The leftover backing the *current* process is still locked and will fail
        // here. That is expected: the next run deletes it.
        let _ = std::fs::remove_file(entry.path());
    }
    Ok(())
}

/// Changes the icon of the running executable, permanently.
///
/// Returns once the file on disk carries the new icon. An Explorer window that was
/// already displaying the file may keep showing the old icon until Explorer is
/// restarted; that cache lives in another process, keyed by path, and nothing the
/// crate can call reliably clears it.
pub fn set_icon<I: Icon>(icon: I) -> Result<(), Error> {
    let exe = win::current_exe()?;
    let (dir, stem) = location(&exe)?;

    let _lock = SwapLock::acquire(&exe)?;
    ensure_writable(dir)?;

    // Parse and validate the whole icon before touching anything on disk, so a
    // truncated file cannot leave the caller's binary half-written.
    let images = ico::parse(icon.bytes())?;
    let group = ico::group_directory(&images, FIRST_ICON_ID);

    // Which icon resources the binary uses today. Any of them the new set does not
    // reuse has to go, or every swap would leave dead images behind and the file
    // would grow without bound.
    let stale = stale_icon_ids(&exe, images.len())?;

    let staging = unique_path(dir, &stem, "new");
    std::fs::copy(&exe, &staging)?;

    // From here on, failure must not leave the staging file behind.
    match write_resources(&staging, &images, &group, &stale, icon.name()) {
        Ok(()) => {}
        Err(err) => {
            let _ = std::fs::remove_file(&staging);
            return Err(err);
        }
    }

    install(&staging, &exe, dir, &stem)?;
    remember_icon(icon.name());
    win::notify_shell();
    Ok(())
}

/// The name of the icon currently on the executable, if the crate put one there.
///
/// Read from the file on disk rather than from the loaded module: after a swap the
/// running image is still mapped from the pre-swap bytes, so the loaded module would
/// answer with the previous icon.
pub fn current_icon() -> Result<Option<String>, Error> {
    let mut cache = icon_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if cache.loaded {
        return Ok(cache.name.clone());
    }

    let exe = win::current_exe()?;
    let name = resources::read_state(&exe)?;
    cache.loaded = true;
    cache.name = name.clone();
    Ok(name)
}

#[derive(Default)]
struct IconCache {
    loaded: bool,
    name: Option<String>,
}

fn icon_cache() -> &'static Mutex<IconCache> {
    static CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(IconCache::default()))
}

fn remember_icon(name: &str) {
    let mut cache = icon_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    cache.loaded = true;
    cache.name = Some(name.to_owned());
}

fn write_resources(
    staging: &Path,
    images: &[ico::Image<'_>],
    group: &[u8],
    stale: &[u16],
    name: &str,
) -> Result<(), Error> {
    let update = Update::begin(staging)?;
    for (index, image) in images.iter().enumerate() {
        update.set_icon_image(FIRST_ICON_ID + index as u16, image.data)?;
    }
    for id in stale {
        update.remove_icon_image(*id)?;
    }
    update.set_group(group)?;
    update.set_state(name)?;
    update.commit()
}

/// Puts `staging` at the executable's path.
///
/// The happy path is a single atomic replace. It only fails while the path is still
/// held by the running image, which is the case exactly once per run — the first
/// swap. After that the process is backed by the renamed file and the path is free.
fn install(staging: &Path, exe: &Path, dir: &Path, stem: &str) -> Result<(), Error> {
    match win::replace(staging, exe) {
        Ok(()) => return Ok(()),
        Err(Error::Windows(_, code))
            if code == ERROR_SHARING_VIOLATION || code == ERROR_ACCESS_DENIED => {}
        Err(err) => {
            let _ = std::fs::remove_file(staging);
            return Err(err);
        }
    }

    // The path is still occupied by the running image. Step aside, then move in.
    // The suffix is unique per swap: a fixed `.old` would collide on the second
    // swap of the same run, and would clobber a file still needed for recovery.
    let parked = unique_path(dir, stem, "old");
    win::rename_new(exe, &parked)?;

    if let Err(err) = win::rename_new(staging, exe) {
        // Put the original back rather than leaving the path empty.
        let _ = win::rename_new(&parked, exe);
        let _ = std::fs::remove_file(staging);
        return Err(err);
    }
    Ok(())
}

fn stale_icon_ids(exe: &Path, keeping: usize) -> Result<Vec<u16>, Error> {
    let Some(group) = resources::read_group(exe)? else {
        return Ok(Vec::new());
    };
    let highest_kept = FIRST_ICON_ID + keeping as u16;
    Ok(ico::referenced_ids(&group)
        .into_iter()
        .filter(|id| *id >= highest_kept)
        .collect())
}

fn ensure_writable(dir: &Path) -> Result<(), Error> {
    // Probing by opening the executable itself would be wrong: that always fails
    // while the process is running, so it would report every directory as read-only.
    // The directory is what actually has to be writable, and probing it also proves
    // the staging file will land on the same volume.
    let probe = unique_path(dir, "alt-icons", "probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(_) => Err(Error::PathNotWritable(dir.to_path_buf())),
    }
}

fn location(exe: &Path) -> Result<(&Path, String), Error> {
    let dir = exe
        .parent()
        .ok_or_else(|| Error::PathNotWritable(exe.to_path_buf()))?;
    let stem = exe
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::PathNotWritable(exe.to_path_buf()))?
        .to_owned();
    Ok((dir, stem))
}

/// `app.exe.1234-1.old`, unique per process and per call.
fn unique_path(dir: &Path, stem: &str, extension: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = unsafe { GetCurrentProcessId() };
    dir.join(format!("{stem}.{pid}-{serial}.{extension}"))
}

/// Matches the leftovers [`unique_path`] produces, without matching the executable
/// itself or anything a user put there.
fn is_leftover(name: &str, stem: &str) -> bool {
    let Some(rest) = name.strip_prefix(stem) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('.') else {
        return false;
    };
    let Some(serial) = rest
        .strip_suffix(".old")
        .or_else(|| rest.strip_suffix(".new"))
    else {
        return false;
    };
    matches!(serial.split_once('-'), Some((pid, count))
        if !pid.is_empty()
            && !count.is_empty()
            && pid.bytes().all(|b| b.is_ascii_digit())
            && count.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::is_leftover;

    #[test]
    fn recognises_its_own_leftovers() {
        assert!(is_leftover("app.exe.1234-0.old", "app.exe"));
        assert!(is_leftover("app.exe.1234-7.new", "app.exe"));
    }

    #[test]
    fn leaves_everything_else_alone() {
        assert!(!is_leftover("app.exe", "app.exe"));
        assert!(!is_leftover("app.exe.old", "app.exe"));
        assert!(!is_leftover("app.exe.backup-0.old", "app.exe"));
        assert!(!is_leftover("notes.txt", "app.exe"));
        assert!(!is_leftover("other.exe.1234-0.old", "app.exe"));
    }
}
