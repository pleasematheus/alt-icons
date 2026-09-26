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
            let data = LockResource(handle) as *const u8;
            if data.is_null() || size == 0 {
                None
            } else {
                Some(std::slice::from_raw_parts(data, size).to_vec())
            }
        };
        FreeLibrary(module);
        found
    }
}
