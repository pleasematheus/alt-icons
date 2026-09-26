//! Parsing of `.ico` files and of the `RT_GROUP_ICON` directory that Windows keeps
//! inside a PE.
//!
//! The two formats are almost the same, with one difference that matters: an
//! `ICONDIRENTRY` in a file ends with a 4-byte offset into that file, while a
//! `GRPICONDIRENTRY` in a binary ends with a 2-byte `RT_ICON` resource id. Building
//! one from the other is the whole job here.

use crate::Error;

const FILE_HEADER_LEN: usize = 6;
const FILE_ENTRY_LEN: usize = 16;
const GROUP_ENTRY_LEN: usize = 14;

/// One image inside an icon file, borrowed from the caller's bytes.
pub struct Image<'a> {
    pub width: u8,
    pub height: u8,
    pub colors: u8,
    pub reserved: u8,
    pub planes: u16,
    pub bpp: u16,
    pub data: &'a [u8],
}

/// Parses a complete `.ico`, validating every entry before returning.
///
/// Nothing is written to a binary until this has succeeded, which is what keeps a
/// truncated or malformed icon from leaving half-written resources inside the
/// caller's own executable.
pub fn parse(bytes: &[u8]) -> Result<Vec<Image<'_>>, Error> {
    if bytes.len() < FILE_HEADER_LEN {
        return Err(Error::MalformedIcon("shorter than an icon header"));
    }
    if u16(bytes, 0) != 0 {
        return Err(Error::MalformedIcon("reserved field is not zero"));
    }
    if u16(bytes, 2) != 1 {
        return Err(Error::MalformedIcon("not an icon (type is not 1)"));
    }

    let count = u16(bytes, 4) as usize;
    if count == 0 {
        return Err(Error::MalformedIcon("contains no images"));
    }

    let directory_end = FILE_HEADER_LEN
        .checked_add(count.checked_mul(FILE_ENTRY_LEN).ok_or(TOO_MANY)?)
        .ok_or(TOO_MANY)?;
    if bytes.len() < directory_end {
        return Err(Error::MalformedIcon("directory is truncated"));
    }

    let mut images = Vec::with_capacity(count);
    for index in 0..count {
        let at = FILE_HEADER_LEN + index * FILE_ENTRY_LEN;
        let size = u32(bytes, at + 8) as usize;
        let offset = u32(bytes, at + 12) as usize;

        if size == 0 {
            return Err(Error::MalformedIcon("an image is empty"));
        }
        let end = offset
            .checked_add(size)
            .ok_or(Error::MalformedIcon("an image offset overflows"))?;
        if end > bytes.len() {
            return Err(Error::MalformedIcon(
                "an image runs past the end of the file",
            ));
        }

        images.push(Image {
            width: bytes[at],
            height: bytes[at + 1],
            colors: bytes[at + 2],
            reserved: bytes[at + 3],
            planes: u16(bytes, at + 4),
            bpp: u16(bytes, at + 6),
            data: &bytes[offset..end],
        });
    }
    Ok(images)
}

const TOO_MANY: Error = Error::MalformedIcon("declares more images than can fit");

/// Builds the `RT_GROUP_ICON` payload for `images`, where image `i` will be stored
/// as `RT_ICON` resource `first_id + i`.
pub fn group_directory(images: &[Image<'_>], first_id: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(FILE_HEADER_LEN + images.len() * GROUP_ENTRY_LEN);
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());

    for (index, image) in images.iter().enumerate() {
        out.push(image.width);
        out.push(image.height);
        out.push(image.colors);
        out.push(image.reserved);
        out.extend_from_slice(&image.planes.to_le_bytes());
        out.extend_from_slice(&image.bpp.to_le_bytes());
        out.extend_from_slice(&(image.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(first_id + index as u16).to_le_bytes());
    }
    out
}

/// Reads back the `RT_ICON` ids a group directory refers to.
///
/// Used before a swap to find which icon resources the binary is currently using, so
/// the ones the new set does not reuse can be deleted instead of accumulating.
pub fn referenced_ids(group: &[u8]) -> Vec<u16> {
    if group.len() < FILE_HEADER_LEN {
        return Vec::new();
    }
    let count = u16(group, 4) as usize;
    let mut ids = Vec::with_capacity(count);
    for index in 0..count {
        let at = FILE_HEADER_LEN + index * GROUP_ENTRY_LEN;
        if at + GROUP_ENTRY_LEN > group.len() {
            break;
        }
        ids.push(u16(group, at + 12));
    }
    ids
}

fn u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but well-formed one-image icon.
    fn sample() -> Vec<u8> {
        let mut ico = vec![0, 0, 1, 0, 1, 0];
        ico.extend_from_slice(&[16, 16, 0, 0]); // 16x16, no palette
        ico.extend_from_slice(&1u16.to_le_bytes()); // planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bpp
        ico.extend_from_slice(&4u32.to_le_bytes()); // size
        ico.extend_from_slice(&22u32.to_le_bytes()); // offset
        ico.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        ico
    }

    #[test]
    fn parses_a_well_formed_icon() {
        let bytes = sample();
        let images = parse(&bytes).expect("sample icon is valid");
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].width, 16);
        assert_eq!(images[0].bpp, 32);
        assert_eq!(images[0].data, &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn rejects_an_image_that_runs_past_the_end() {
        let mut bytes = sample();
        bytes.truncate(bytes.len() - 1);
        assert!(matches!(parse(&bytes), Err(Error::MalformedIcon(_))));
    }

    #[test]
    fn rejects_a_cursor() {
        let mut bytes = sample();
        bytes[2] = 2; // type 2 is a cursor
        assert!(matches!(parse(&bytes), Err(Error::MalformedIcon(_))));
    }

    #[test]
    fn group_directory_round_trips_through_referenced_ids() {
        let bytes = sample();
        let images = parse(&bytes).expect("sample icon is valid");
        let group = group_directory(&images, 1);
        assert_eq!(referenced_ids(&group), vec![1]);
    }
}
