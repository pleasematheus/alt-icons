//! Thin wrappers over the Win32 calls the crate needs. Nothing here knows about
//! icons; it is all handles, paths and strings.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use base64::Engine;

use windows_sys::Win32::Foundation::{
    CloseHandle, SetLastError, ERROR_ALREADY_EXISTS, FILETIME, HANDLE, MAX_PATH,
};
use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, CreateProcessW, GetCurrentProcess, GetCurrentProcessId, GetProcessTimes,
    ReleaseMutex, CREATE_NO_WINDOW, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
};
use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

use crate::Error;

/// A NUL-terminated UTF-16 buffer, kept alive for as long as the pointer is used.
pub struct Wide(Vec<u16>);

impl Wide {
    pub fn new(text: impl AsRef<std::ffi::OsStr>) -> Self {
        let mut buffer: Vec<u16> = text.as_ref().encode_wide().collect();
        buffer.push(0);
        Wide(buffer)
    }

    pub fn as_ptr(&self) -> *const u16 {
        self.0.as_ptr()
    }
}

/// Full path of the running executable.
pub fn current_exe() -> Result<PathBuf, Error> {
    static LAUNCH_PATH: OnceLock<PathBuf> = OnceLock::new();

    if let Some(path) = LAUNCH_PATH.get() {
        return Ok(path.clone());
    }

    let path = query_current_exe()?;
    Ok(LAUNCH_PATH.get_or_init(|| path).clone())
}

fn query_current_exe() -> Result<PathBuf, Error> {
    // `std::env::current_exe` resolves symlinks and can hand back a different path
    // than the one the process was launched from. The icon lives at the launched
    // path, so ask Windows directly. Cache this first result: after the running
    // image is renamed aside, GetModuleFileNameW starts returning the parked path.
    let mut buffer = vec![0u16; MAX_PATH as usize];
    loop {
        let written = unsafe {
            GetModuleFileNameW(
                std::ptr::null_mut(),
                buffer.as_mut_ptr(),
                buffer.len() as u32,
            )
        };
        if written == 0 {
            return Err(Error::Windows("GetModuleFileNameW", last_error()));
        }
        if (written as usize) < buffer.len() {
            buffer.truncate(written as usize);
            return Ok(PathBuf::from(OsString::from_wide(&buffer)));
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

/// Renames `from` to `to`, replacing `to` if it exists.
///
/// On the same volume this is atomic, which is why the crate insists on putting its
/// temporary file in the directory of the executable rather than in `%TEMP%`.
pub fn replace(from: &Path, to: &Path) -> Result<(), Error> {
    rename(from, to, MOVEFILE_REPLACE_EXISTING)
}

/// Renames `from` to `to`, failing if `to` already exists.
pub fn rename_new(from: &Path, to: &Path) -> Result<(), Error> {
    rename(from, to, 0)
}

fn rename(from: &Path, to: &Path, flags: u32) -> Result<(), Error> {
    let from_w = Wide::new(from);
    let to_w = Wide::new(to);
    let ok = unsafe { MoveFileExW(from_w.as_ptr(), to_w.as_ptr(), flags) };
    if ok == 0 {
        return Err(Error::Windows("MoveFileExW", last_error()));
    }
    Ok(())
}

/// Tells the shell that file associations changed, which nudges open Explorer
/// windows to redraw.
///
/// This is a courtesy, not the mechanism: the icon of the file on disk changes
/// whether or not this is called. See `DESIGN.md` for what was measured.
pub fn notify_shell() {
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
}

/// A named mutex that serialises swaps of one executable across processes.
///
/// The name is derived from the executable's path, so two copies of the same binary
/// in different folders do not block each other. Windows destroys a named mutex when
/// the last handle closes, so a crashed process cannot leave one held forever.
pub struct SwapLock(HANDLE);

impl SwapLock {
    pub fn acquire(exe: &Path) -> Result<Self, Error> {
        let name = format!("Local\\alt-icons-{:016x}", path_hash(exe));
        let name_w = Wide::new(name);
        // `CreateMutexW` sets ERROR_ALREADY_EXISTS when the named object was already
        // there, but is not documented to clear the slot when it was not. Without
        // this, a stale code left by any earlier call on this thread would surface
        // as a spurious `SwapInProgress`.
        unsafe { SetLastError(0) };
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name_w.as_ptr()) };
        if handle.is_null() {
            return Err(Error::Windows("CreateMutexW", last_error()));
        }
        if last_error() == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return Err(Error::SwapInProgress);
        }
        Ok(SwapLock(handle))
    }
}

impl Drop for SwapLock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

pub fn last_error() -> u32 {
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}

/// A detached system helper outlives the image being swapped. All paths are passed
/// as PowerShell literal strings, never interpolated as executable commands.
pub fn schedule_cleanup(path: &Path, exe: &Path, wait_for_current: bool) -> Result<(), Error> {
    let mut system_dir = vec![0u16; MAX_PATH as usize];
    let written = unsafe { GetSystemDirectoryW(system_dir.as_mut_ptr(), system_dir.len() as u32) };
    if written == 0 || written as usize >= system_dir.len() {
        return Err(Error::Windows("GetSystemDirectoryW", last_error()));
    }
    system_dir.truncate(written as usize);
    let powershell = PathBuf::from(OsString::from_wide(&system_dir))
        .join("WindowsPowerShell/v1.0/powershell.exe");

    let (owner, started) = if wait_for_current {
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        } == 0
        {
            return Err(Error::Windows("GetProcessTimes", last_error()));
        }
        (
            unsafe { GetCurrentProcessId() },
            (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime),
        )
    } else {
        (0, 0)
    };
    let path = path.as_os_str().to_string_lossy().replace('\'', "''");
    let exe = exe.as_os_str().to_string_lossy().replace('\'', "''");
    let script = format!(
        "$ownerId = {owner}; $ownerStarted = {started}; $oldPath = '{path}'; $launchPath = '{exe}';\n{}",
        include_str!("cleanup.ps1")
    );
    // EncodedCommand takes base64 of UTF-16LE, avoiding command-line quoting for
    // script contents. The application path is supplied separately to Windows.
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let application = Wide::new(&powershell);
    let mut command = Wide::new(format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}"
    ));
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESTDHANDLES,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // Inherit NO handles, including incidental handles unrelated to standard IO.
    // Otherwise the helper could keep another child process's stdin pipe open or
    // retain application locks until it exits. All three std handles are null.
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.0.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut process,
        )
    } == 0
    {
        return Err(Error::Windows("CreateProcessW", last_error()));
    }
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

/// Case-insensitive FNV-1a over the path, so the lock name is stable for a given
/// executable regardless of how the path was capitalised.
fn path_hash(path: &Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for unit in path.as_os_str().encode_wide() {
        let folded = if (b'A' as u16..=b'Z' as u16).contains(&unit) {
            unit + 32
        } else {
            unit
        };
        hash ^= folded as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
