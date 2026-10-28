//! Previous unchecked readers. Retained only for reference.
use super::*;

#[deprecated(since = "1.2.9", note = "Use win::lookup_icon_id() instead.")]
pub fn lookup_icon_id(group: &[u8], size: i32) -> u16 {
    unsafe { LookupIconIdFromDirectoryEx(group.as_ptr(), 1, size, size, 0) as u16 }
}

#[deprecated(since = "1.2.9", note = "Use win::read() instead.")]
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
