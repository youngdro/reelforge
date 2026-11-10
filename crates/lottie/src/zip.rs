//! Just enough of the ZIP format to read a dotLottie file: stored and deflated entries.

fn u16_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn u32_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// The files of an archive: their names and where to find them.
pub(crate) struct Archive<'a> {
    bytes: &'a [u8],
    /// Name, method, packed size, unpacked size, local header offset.
    entries: Vec<(String, usize, usize, usize, usize)>,
}

/// The most one entry may unpack to.
const LIMIT: usize = 256 << 20;

impl<'a> Archive<'a> {
    pub fn is(bytes: &[u8]) -> bool {
        bytes.starts_with(b"PK\x03\x04")
    }

    pub fn read(bytes: &'a [u8]) -> Option<Archive<'a>> {
        // the directory's end is the last thing in the file, but for a comment
        let tail = bytes.len().checked_sub(22)?;
        let end = (tail.saturating_sub(65_536)..=tail).rev().find(|at| bytes[*at..].starts_with(b"PK\x05\x06"))?;
        let (count, mut at) = (u16_at(bytes, end + 10)?, u32_at(bytes, end + 16)?);
        let mut entries = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            if !bytes.get(at..)?.starts_with(b"PK\x01\x02") {
                return None;
            }
            let (method, packed, unpacked) =
                (u16_at(bytes, at + 10)?, u32_at(bytes, at + 20)?, u32_at(bytes, at + 24)?);
            let (name, extra, comment) = (u16_at(bytes, at + 28)?, u16_at(bytes, at + 30)?, u16_at(bytes, at + 32)?);
            let offset = u32_at(bytes, at + 42)?;
            let text = String::from_utf8_lossy(bytes.get(at + 46..at + 46 + name)?).into_owned();
            entries.push((text, method, packed, unpacked, offset));
            at += 46 + name + extra + comment;
        }
        Some(Archive { bytes, entries })
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.0.as_str())
    }

    pub fn file(&self, name: &str) -> Option<Vec<u8>> {
        let (_, method, packed, unpacked, offset) = self.entries.iter().find(|e| e.0 == name)?;
        let b = self.bytes;
        if !b.get(*offset..)?.starts_with(b"PK\x03\x04") {
            return None;
        }
        let start = offset + 30 + u16_at(b, offset + 26)? + u16_at(b, offset + 28)?;
        let data = b.get(start..start + packed)?;
        match method {
            0 => Some(data.to_vec()),
            8 if *unpacked <= LIMIT => miniz_oxide::inflate::decompress_to_vec_with_limit(data, LIMIT).ok(),
            _ => None,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A ZIP of stored and deflated files, written by hand.
    pub(crate) fn archive(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for (name, data, deflate) in files {
            let packed = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let offset = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            let mut head = Vec::new();
            head.extend(method.to_le_bytes());
            head.extend([0u8; 8]); // time, date, crc
            head.extend((packed.len() as u32).to_le_bytes());
            head.extend((data.len() as u32).to_le_bytes());
            head.extend((name.len() as u16).to_le_bytes());
            head.extend(0u16.to_le_bytes());
            out.extend(b"PK\x03\x04\x14\x00\x00\x00");
            out.extend(&head);
            out.extend(name.as_bytes());
            out.extend(&packed);
            directory.extend(b"PK\x01\x02\x14\x00\x14\x00\x00\x00");
            directory.extend(&head);
            directory.extend([0u8; 10]); // comment length, disk, attributes
            directory.extend(offset.to_le_bytes());
            directory.extend(name.as_bytes());
        }
        let at = out.len() as u32;
        out.extend(&directory);
        out.extend(b"PK\x05\x06\x00\x00\x00\x00");
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((directory.len() as u32).to_le_bytes());
        out.extend(at.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out
    }

    #[test]
    fn files_are_read_stored_or_deflated() {
        let text = b"hello hello hello hello hello".as_slice();
        let zip = archive(&[("a.txt", text, false), ("dir/b.txt", text, true)]);
        assert!(Archive::is(&zip));
        let a = Archive::read(&zip).unwrap();
        assert_eq!(a.names().collect::<Vec<_>>(), ["a.txt", "dir/b.txt"]);
        assert_eq!(a.file("a.txt").unwrap(), text);
        assert_eq!(a.file("dir/b.txt").unwrap(), text);
        assert!(a.file("c.txt").is_none() && Archive::read(b"not a zip at all, just words").is_none());
    }
}
