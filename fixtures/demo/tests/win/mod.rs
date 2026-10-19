//! The bits of Win32 the integration test needs to inspect a binary from outside
//! the crate, so the test is not just asking `alt-icons` to confirm itself.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::FreeLibrary;
use windows_sys::Win32::System::LibraryLoader::{
    FindResourceW, LoadLibraryExW, LoadResource, LockResource, SizeofResource,
    LOAD_LIBRARY_AS_DATAFILE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::LookupIconIdFromDirectoryEx;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const GROUP_ID: u16 = 1;

#[allow(dead_code, deprecated)]
mod legacy;

pub fn read_group(exe: &Path) -> Option<Vec<u8>> {
    read(exe, RT_GROUP_ICON, GROUP_ID)
}

pub fn read_icon(exe: &Path, id: u16) -> Option<Vec<u8>> {
    read(exe, RT_ICON, id)
}

/// Asks Windows which `RT_ICON` of the group it would use at the given size. This is
/// the same resolution the shell performs, which is why the test uses it rather than
/// picking an entry itself.
pub fn lookup_icon_id(group: &[u8], size: i32) -> u16 {
    if size < 0 || group.len() < 6 || group[..4] != [0, 0, 1, 0] {
        return 0;
    }
    let count = u16::from_le_bytes([group[4], group[5]]) as usize;
    if count == 0 || group.len() < 6 + count * 14 {
        return 0;
    }
    // SAFETY: The header and all declared GRPICONDIRENTRY records fit in the live buffer.
    unsafe { LookupIconIdFromDirectoryEx(group.as_ptr(), 1, size, size, 0) as u16 }
}

fn read(exe: &Path, kind: u16, name: u16) -> Option<Vec<u8>> {
    let mut wide: Vec<u16> = exe.as_os_str().encode_wide().collect();
    wide.push(0);

    unsafe {
        let module = LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_AS_DATAFILE,
        );
        if module.is_null() {
            return None;
        }
        let info = FindResourceW(
            module,
            name as usize as *const u16,
            kind as usize as *const u16,
        );
        let found = if info.is_null() {
            None
        } else {
            let size = SizeofResource(module, info) as usize;
            let handle = LoadResource(module, info);
            let data = if handle.is_null() {
                std::ptr::null()
            } else {
                LockResource(handle) as *const u8
            };
            if data.is_null() || size == 0 || size > isize::MAX as usize {
                None
            } else {
                Some(std::slice::from_raw_parts(data, size).to_vec())
            }
        };
        FreeLibrary(module);
        found
    }
}

#[test]
fn rejects_short_icon_directory_headers() {
    for size in 0..6 {
        assert_eq!(lookup_icon_id(&[0; 6][..size], 32), 0);
    }
}

#[test]
fn rejects_truncated_icon_directory_entries() {
    let mut group = vec![0, 0, 1, 0, 1, 0];
    group.extend_from_slice(&[0; 13]);
    assert_eq!(lookup_icon_id(&group, 32), 0);
    assert_eq!(lookup_icon_id(&[0, 0, 1, 0, 255, 255], 32), 0);
}

#[test]
fn rejects_empty_or_non_icon_directories() {
    assert_eq!(lookup_icon_id(&[0, 0, 1, 0, 0, 0], 32), 0);
    assert_eq!(lookup_icon_id(&[0, 0, 2, 0, 1, 0], 32), 0);
}
