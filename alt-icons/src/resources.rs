//! Reading and writing PE resources.
//!
//! Reading goes through `LoadLibraryExW` with `LOAD_LIBRARY_AS_DATAFILE`, so a file
//! can be inspected without being executed or mapped as code. Writing goes through
//! the `BeginUpdateResource` family, which happily creates a resource section in a
//! binary that had none — a freshly built Rust executable, for instance.

use std::path::Path;

use windows_sys::Win32::Foundation::{FreeLibrary, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    BeginUpdateResourceW, EndUpdateResourceW, FindResourceW, LoadLibraryExW, LoadResource,
    LockResource, SizeofResource, UpdateResourceW, LOAD_LIBRARY_AS_DATAFILE,
};

use crate::win::{last_error, Wide};
use crate::Error;

pub const RT_ICON: u16 = 3;
pub const RT_RCDATA: u16 = 10;
pub const RT_GROUP_ICON: u16 = 14;

/// The single icon group the crate owns. `alt-icons-build` bakes the default icon
/// under this same id, so there is never more than one group and never any question
/// about which one the shell picks.
pub const GROUP_ID: u16 = 1;

/// `RT_ICON` ids are handed out from here, one per image in the `.ico`.
pub const FIRST_ICON_ID: u16 = 1;

/// Neutral language, matching the `LANGUAGE 0, 0` the build script writes. Keeping
/// both sides on one language means an update always replaces the baked resource
/// instead of sitting beside it as a second translation.
pub const LANGUAGE: u16 = 0;

/// Name of the resource holding which icon is currently active.
pub const STATE_NAME: &str = "ALT_ICONS_STATE";

/// A resource type or name that is a plain integer rather than a string.
fn int_resource(id: u16) -> *const u16 {
    id as usize as *const u16
}

/// Reads one resource out of a PE file on disk.
///
/// Deliberately reads the *file*, never the loaded module: after a swap the running
/// image is still mapped from the pre-swap bytes, so asking the loaded module would
/// return stale data.
pub fn read(path: &Path, kind: *const u16, name: *const u16) -> Result<Option<Vec<u8>>, Error> {
    let path_w = Wide::new(path);
    let module: HMODULE = unsafe {
        LoadLibraryExW(
            path_w.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_AS_DATAFILE,
        )
    };
    if module.is_null() {
        return Err(Error::Windows("LoadLibraryExW", last_error()));
    }

    let found = unsafe {
        let info = FindResourceW(module, name, kind);
        if info.is_null() {
            None
        } else {
            let size = SizeofResource(module, info) as usize;
            let handle = LoadResource(module, info);
            let data = LockResource(handle) as *const u8;
            if handle.is_null() || data.is_null() || size == 0 {
                None
            } else {
                Some(std::slice::from_raw_parts(data, size).to_vec())
            }
        }
    };

    unsafe { FreeLibrary(module) };
    Ok(found)
}

pub fn read_group(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    read(path, int_resource(RT_GROUP_ICON), int_resource(GROUP_ID))
}

pub fn read_state(path: &Path) -> Result<Option<String>, Error> {
    let name = Wide::new(STATE_NAME);
    let bytes = read(path, int_resource(RT_RCDATA), name.as_ptr())?;
    Ok(bytes.and_then(|bytes| String::from_utf8(bytes).ok()))
}

/// An open resource-update transaction. Dropping without [`Update::commit`] discards
/// every pending change, so an early return cannot leave a half-written binary.
pub struct Update {
    handle: *mut std::ffi::c_void,
    committed: bool,
}

impl Update {
    pub fn begin(path: &Path) -> Result<Self, Error> {
        let path_w = Wide::new(path);
        let handle = unsafe { BeginUpdateResourceW(path_w.as_ptr(), 0) };
        if handle.is_null() {
            return Err(Error::Windows("BeginUpdateResourceW", last_error()));
        }
        Ok(Update {
            handle,
            committed: false,
        })
    }

    pub fn set(&self, kind: *const u16, name: *const u16, data: &[u8]) -> Result<(), Error> {
        let ok = unsafe {
            UpdateResourceW(
                self.handle,
                kind,
                name,
                LANGUAGE,
                data.as_ptr() as *const std::ffi::c_void,
                data.len() as u32,
            )
        };
        if ok == 0 {
            return Err(Error::Windows("UpdateResourceW", last_error()));
        }
        Ok(())
    }

    /// Removes a resource. A null data pointer is how Win32 spells "delete".
    pub fn remove(&self, kind: *const u16, name: *const u16) -> Result<(), Error> {
        let ok = unsafe { UpdateResourceW(self.handle, kind, name, LANGUAGE, std::ptr::null(), 0) };
        if ok == 0 {
            return Err(Error::Windows("UpdateResourceW (delete)", last_error()));
        }
        Ok(())
    }

    pub fn set_icon_image(&self, id: u16, data: &[u8]) -> Result<(), Error> {
        self.set(int_resource(RT_ICON), int_resource(id), data)
    }

    pub fn remove_icon_image(&self, id: u16) -> Result<(), Error> {
        self.remove(int_resource(RT_ICON), int_resource(id))
    }

    pub fn set_group(&self, data: &[u8]) -> Result<(), Error> {
        self.set(int_resource(RT_GROUP_ICON), int_resource(GROUP_ID), data)
    }

    pub fn set_state(&self, name: &str) -> Result<(), Error> {
        let resource_name = Wide::new(STATE_NAME);
        self.set(
            int_resource(RT_RCDATA),
            resource_name.as_ptr(),
            name.as_bytes(),
        )
    }

    pub fn commit(mut self) -> Result<(), Error> {
        let ok = unsafe { EndUpdateResourceW(self.handle, 0) };
        self.committed = true;
        if ok == 0 {
            return Err(Error::Windows("EndUpdateResourceW", last_error()));
        }
        Ok(())
    }
}

impl Drop for Update {
    fn drop(&mut self) {
        if !self.committed {
            // Discard: the second argument is `fDiscard`.
            unsafe { EndUpdateResourceW(self.handle, 1) };
        }
    }
}
