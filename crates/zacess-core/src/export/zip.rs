//! A small ZIP writer for EPUB containers.
//!
//! Entries are stored rather than compressed: an EPUB's `mimetype` entry must be stored anyway,
//! the text inside an EPUB is small and a stored archive needs no compression dependency. The
//! output is deterministic, with a fixed timestamp, so the same document always gives the same
//! bytes.

/// A fixed DOS date, 1 January 2026, so archives do not change with the clock.
const DOS_DATE: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;
const DOS_TIME: u16 = 0;
/// General purpose flag bit 11: file names are UTF-8.
const UTF8_NAMES: u16 = 0x0800;

struct Entry {
    name: Vec<u8>,
    crc: u32,
    size: u32,
    offset: u32,
}

#[derive(Default)]
pub struct ZipWriter {
    data: Vec<u8>,
    entries: Vec<Entry>,
}

impl ZipWriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a stored entry. Names use forward slashes and no leading slash.
    pub fn add(&mut self, name: &str, bytes: &[u8]) {
        let name = name.as_bytes().to_vec();
        let crc = crc32(bytes);
        let size = bytes.len() as u32;
        let offset = self.data.len() as u32;
        self.data.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.data.extend_from_slice(&20u16.to_le_bytes());
        self.data.extend_from_slice(&UTF8_NAMES.to_le_bytes());
        self.data.extend_from_slice(&0u16.to_le_bytes());
        self.data.extend_from_slice(&DOS_TIME.to_le_bytes());
        self.data.extend_from_slice(&DOS_DATE.to_le_bytes());
        self.data.extend_from_slice(&crc.to_le_bytes());
        self.data.extend_from_slice(&size.to_le_bytes());
        self.data.extend_from_slice(&size.to_le_bytes());
        self.data
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.data.extend_from_slice(&0u16.to_le_bytes());
        self.data.extend_from_slice(&name);
        self.data.extend_from_slice(bytes);
        self.entries.push(Entry {
            name,
            crc,
            size,
            offset,
        });
    }

    pub fn finish(mut self) -> Vec<u8> {
        let central_start = self.data.len() as u32;
        for e in &self.entries {
            self.data.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            self.data.extend_from_slice(&20u16.to_le_bytes());
            self.data.extend_from_slice(&20u16.to_le_bytes());
            self.data.extend_from_slice(&UTF8_NAMES.to_le_bytes());
            self.data.extend_from_slice(&0u16.to_le_bytes());
            self.data.extend_from_slice(&DOS_TIME.to_le_bytes());
            self.data.extend_from_slice(&DOS_DATE.to_le_bytes());
            self.data.extend_from_slice(&e.crc.to_le_bytes());
            self.data.extend_from_slice(&e.size.to_le_bytes());
            self.data.extend_from_slice(&e.size.to_le_bytes());
            self.data
                .extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            self.data.extend_from_slice(&0u16.to_le_bytes());
            self.data.extend_from_slice(&0u16.to_le_bytes());
            self.data.extend_from_slice(&0u16.to_le_bytes());
            self.data.extend_from_slice(&0u16.to_le_bytes());
            self.data.extend_from_slice(&0u32.to_le_bytes());
            self.data.extend_from_slice(&e.offset.to_le_bytes());
            self.data.extend_from_slice(&e.name);
        }
        let central_size = self.data.len() as u32 - central_start;
        let count = self.entries.len() as u16;
        self.data.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.data.extend_from_slice(&0u16.to_le_bytes());
        self.data.extend_from_slice(&0u16.to_le_bytes());
        self.data.extend_from_slice(&count.to_le_bytes());
        self.data.extend_from_slice(&count.to_le_bytes());
        self.data.extend_from_slice(&central_size.to_le_bytes());
        self.data.extend_from_slice(&central_start.to_le_bytes());
        self.data.extend_from_slice(&0u16.to_le_bytes());
        self.data
    }
}

/// CRC-32 as ZIP uses it, polynomial 0xEDB88320.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_reference_value() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"application/epub+zip"), 0x2cab_616f);
    }

    #[test]
    fn the_first_entry_starts_at_byte_zero_and_is_stored() {
        let mut zip = ZipWriter::new();
        zip.add("mimetype", b"application/epub+zip");
        zip.add("META-INF/container.xml", b"<container/>");
        let bytes = zip.finish();
        assert_eq!(&bytes[0..4], &[0x50, 0x4b, 0x03, 0x04]);
        // method is stored and the name follows the 30-byte header, as EPUB requires
        assert_eq!(&bytes[8..10], &[0, 0]);
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..58], b"application/epub+zip");
        // the archive ends with an end-of-central-directory record naming two entries
        let eocd = bytes.len() - 22;
        assert_eq!(&bytes[eocd..eocd + 4], &[0x50, 0x4b, 0x05, 0x06]);
        assert_eq!(u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]), 2);
    }
}
