//! Reading and writing PE resources.
//!
//! Reading goes through `LoadLibraryExW` with `LOAD_LIBRARY_AS_DATAFILE`, so a file
//! can be inspected without being executed or mapped as code. Writing uses an
//! in-memory PE resource directory and commits only to the staging file.

use std::path::{Path, PathBuf};

use editpe::{
    Image, ResourceData, ResourceDirectory, ResourceEntry, ResourceEntryName, ResourceTable,
};

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

/// The single icon group the crate owns. `alt-icons::build` bakes the default icon
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
    path: PathBuf,
    image: Image<'static>,
    resources: ResourceDirectory,
}

impl Update {
    pub fn begin(path: &Path) -> Result<Self, Error> {
        let image = Image::parse(std::fs::read(path)?).map_err(pe_error)?;
        let resources = image.resource_directory().cloned().unwrap_or_default();
        Ok(Self {
            path: path.to_owned(),
            image,
            resources,
        })
    }

    fn set(&mut self, kind: u16, name: ResourceEntryName, data: &[u8]) -> Result<(), Error> {
        u32::try_from(data.len()).map_err(pe_error)?;
        let types = child_table(
            self.resources.root_mut(),
            ResourceEntryName::ID(kind.into()),
        )?;
        let languages = child_table(types, name)?;
        let mut resource = ResourceData::default();
        resource.set_codepage(0);
        resource.set_data(data.to_vec());
        languages.insert(
            ResourceEntryName::ID(LANGUAGE.into()),
            ResourceEntry::Data(resource),
        );
        Ok(())
    }

    pub fn set_icon_image(&mut self, id: u16, data: &[u8]) -> Result<(), Error> {
        self.set(RT_ICON, ResourceEntryName::ID(id.into()), data)
    }

    pub fn remove_icon_image(&mut self, id: u16) -> Result<(), Error> {
        let kind = ResourceEntryName::ID(RT_ICON.into());
        let name = ResourceEntryName::ID(id.into());
        let Some(types) = self
            .resources
            .root_mut()
            .get_mut(&kind)
            .and_then(ResourceEntry::as_table_mut)
        else {
            return Ok(());
        };
        let Some(languages) = types.get_mut(&name).and_then(ResourceEntry::as_table_mut) else {
            return Ok(());
        };
        languages.remove(ResourceEntryName::ID(LANGUAGE.into()));
        if languages.entries().is_empty() {
            types.remove(&name);
        }
        if types.entries().is_empty() {
            self.resources.root_mut().remove(&kind);
        }
        Ok(())
    }

    pub fn set_group(&mut self, data: &[u8]) -> Result<(), Error> {
        self.set(RT_GROUP_ICON, ResourceEntryName::ID(GROUP_ID.into()), data)
    }

    pub fn set_state(&mut self, name: &str) -> Result<(), Error> {
        self.set(
            RT_RCDATA,
            ResourceEntryName::try_from_string(STATE_NAME).map_err(pe_error)?,
            name.as_bytes(),
        )
    }

    pub fn commit(mut self) -> Result<(), Error> {
        self.image
            .set_resource_directory(self.resources)
            .map_err(pe_error)?;
        // The caller supplies a staging file. The running executable is installed only after success.
        std::fs::write(&self.path, self.image.data())?;
        Ok(())
    }
}

fn child_table(
    table: &mut ResourceTable,
    name: ResourceEntryName,
) -> Result<&mut ResourceTable, Error> {
    if table.get(&name).is_none() {
        table.insert(&name, ResourceEntry::Table(ResourceTable::default()));
    }
    table
        .get_mut(&name)
        .and_then(ResourceEntry::as_table_mut)
        .ok_or_else(|| pe_error("a resource directory contains data where a table is required"))
}

fn pe_error(error: impl std::fmt::Display) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        error.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static SERIAL: AtomicU32 = AtomicU32::new(0);
            let id = SERIAL.fetch_add(1, Ordering::Relaxed);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "alt-icons-resource-{}-{timestamp}-{id}",
                std::process::id()
            ));
            std::fs::create_dir(&dir).unwrap();
            std::fs::copy(std::env::current_exe().unwrap(), dir.join("image.exe")).unwrap();
            Self(dir)
        }

        fn image(&self) -> PathBuf {
            self.0.join("image.exe")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn uncommitted_resources_leave_the_file_unchanged() {
        let scratch = Scratch::new();
        let before = std::fs::read(scratch.image()).unwrap();
        let mut update = Update::begin(&scratch.image()).unwrap();
        update.set_state("pending").unwrap();
        drop(update);
        assert_eq!(std::fs::read(scratch.image()).unwrap(), before);
    }

    #[test]
    fn committed_state_is_readable_by_windows() {
        let scratch = Scratch::new();
        let mut update = Update::begin(&scratch.image()).unwrap();
        update.set_state("ação 🌙").unwrap();
        update.commit().unwrap();
        assert_eq!(
            read_state(&scratch.image()).unwrap().as_deref(),
            Some("ação 🌙")
        );
    }

    #[test]
    fn invalid_image_is_rejected_without_writing() {
        let scratch = Scratch::new();
        let invalid = b"MZ truncated image";
        std::fs::write(scratch.image(), invalid).unwrap();
        assert!(Update::begin(&scratch.image()).is_err());
        assert_eq!(std::fs::read(scratch.image()).unwrap(), invalid);
    }

    #[test]
    fn varying_resource_sizes_do_not_accumulate_sections() {
        let scratch = Scratch::new();
        let baseline = std::fs::metadata(scratch.image()).unwrap().len();
        let mut maximum = baseline;
        for cycle in 0..3 {
            for size in [131_072, 512, 65_536] {
                let mut update = Update::begin(&scratch.image()).unwrap();
                update.set_state(&"x".repeat(size)).unwrap();
                update.commit().unwrap();
                let length = std::fs::metadata(scratch.image()).unwrap().len();
                if cycle == 0 {
                    maximum = maximum.max(length);
                } else {
                    assert!(
                        length <= maximum,
                        "resource sections accumulated after the first cycle"
                    );
                }
            }
        }
        assert!(maximum <= baseline + 131_072 + 4096);
    }

    #[test]
    fn updates_preserve_unrelated_resources_and_other_languages() {
        let scratch = Scratch::new();
        let mut initial = Update::begin(&scratch.image()).unwrap();
        initial
            .set(RT_RCDATA, ResourceEntryName::ID(777), b"unrelated")
            .unwrap();
        initial.set_icon_image(999, b"neutral icon").unwrap();
        let icons = child_table(
            initial.resources.root_mut(),
            ResourceEntryName::ID(RT_ICON.into()),
        )
        .unwrap();
        let languages = child_table(icons, ResourceEntryName::ID(999)).unwrap();
        let mut localized = ResourceData::default();
        localized.set_codepage(1252);
        localized.set_data(b"localized icon".to_vec());
        languages.insert(ResourceEntryName::ID(1046), ResourceEntry::Data(localized));
        initial.commit().unwrap();
        let before = Image::parse_file(scratch.image()).unwrap();

        let mut update = Update::begin(&scratch.image()).unwrap();
        update.remove_icon_image(999).unwrap();
        update.set_state("Dark").unwrap();
        update.commit().unwrap();
        let after = Image::parse_file(scratch.image()).unwrap();
        let before_root = before.resource_directory().unwrap().root();
        let after_root = after.resource_directory().unwrap().root();
        let rcdata = ResourceEntryName::ID(RT_RCDATA.into());
        assert_eq!(
            before_root
                .get(&rcdata)
                .unwrap()
                .as_table()
                .unwrap()
                .get(ResourceEntryName::ID(777)),
            after_root
                .get(&rcdata)
                .unwrap()
                .as_table()
                .unwrap()
                .get(ResourceEntryName::ID(777)),
        );
        let icons = after_root
            .get(ResourceEntryName::ID(RT_ICON.into()))
            .unwrap()
            .as_table()
            .unwrap();
        let languages = icons
            .get(ResourceEntryName::ID(999))
            .unwrap()
            .as_table()
            .unwrap();
        assert!(languages.get(ResourceEntryName::ID(0)).is_none());
        let localized = languages
            .get(ResourceEntryName::ID(1046))
            .unwrap()
            .as_data()
            .unwrap();
        assert_eq!(localized.data(), b"localized icon");
        assert_eq!(localized.codepage(), 1252);
    }
}

// Keep the complete previous transaction for reference, including its rollback path.
#[allow(dead_code, deprecated)]
mod legacy_update {
    use super::*;

    #[deprecated(since = "1.2.8", note = "Use resources::Update instead.")]
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
            let ok =
                unsafe { UpdateResourceW(self.handle, kind, name, LANGUAGE, std::ptr::null(), 0) };
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
}
