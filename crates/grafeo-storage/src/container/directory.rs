//! Section directory: the index of sections within a `.grafeo` container.
//!
//! The directory occupies a single 4 KiB page at offset 0x3000 in the file.
//! It contains a count followed by fixed-size 32-byte entries, one per section.

use grafeo_common::storage::section::{SectionDirectoryEntry, SectionFlags, SectionType};
use grafeo_common::utils::error::{Error, Result};

/// Maximum number of sections in a directory (limited by 4 KiB page).
/// (4096 - 8 byte header) / 32 bytes per entry = 127 entries.
pub const MAX_SECTIONS: usize = 127;

/// Size of the section directory page in bytes.
pub const DIRECTORY_PAGE_SIZE: usize = 4096;

/// Byte offset of the section directory within the `.grafeo` file.
pub const DIRECTORY_OFFSET: u64 = 3 * 4096; // After FileHeader + 2 DbHeaders

/// Byte offset where section data begins (after all headers + directory).
pub const SECTION_DATA_OFFSET: u64 = 4 * 4096;

/// In-memory representation of the section directory.
#[derive(Debug, Clone)]
pub struct SectionDirectory {
    entries: Vec<SectionDirectoryEntry>,
}

impl SectionDirectory {
    /// Create an empty directory.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Number of sections in the directory.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the directory has no sections.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// All entries in the directory.
    #[must_use]
    pub fn entries(&self) -> &[SectionDirectoryEntry] {
        &self.entries
    }

    /// Find an entry by section type.
    #[must_use]
    pub fn find(&self, section_type: SectionType) -> Option<&SectionDirectoryEntry> {
        self.entries.iter().find(|e| e.section_type == section_type)
    }

    /// Add or replace an entry for the given section type.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory is full.
    pub fn upsert(&mut self, entry: SectionDirectoryEntry) -> Result<()> {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|e| e.section_type == entry.section_type)
        {
            *existing = entry;
        } else {
            if self.entries.len() >= MAX_SECTIONS {
                return Err(Error::Internal(format!(
                    "section directory is full ({MAX_SECTIONS} entries)"
                )));
            }
            self.entries.push(entry);
        }
        Ok(())
    }

    /// Remove an entry by section type. Returns true if found.
    pub fn remove(&mut self, section_type: SectionType) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.section_type != section_type);
        self.entries.len() < before
    }

    /// Serialize the directory to a fixed 4 KiB page.
    ///
    /// Layout:
    /// - Bytes 0-3: entry count (u32 LE)
    /// - Bytes 4-7: reserved (zero)
    /// - Bytes 8+: entries (32 bytes each)
    /// - Remaining: zero-padded to 4096 bytes
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; DIRECTORY_PAGE_SIZE];

        // Header: entry count
        let count = self.entries.len() as u32;
        buf[0..4].copy_from_slice(&count.to_le_bytes());
        // Bytes 4-7: reserved

        // Entries
        for (i, entry) in self.entries.iter().enumerate() {
            let offset = 8 + i * SectionDirectoryEntry::SIZE;
            write_entry(
                &mut buf[offset..offset + SectionDirectoryEntry::SIZE],
                entry,
            );
        }

        buf
    }

    /// Deserialize a directory from a 4 KiB page.
    ///
    /// # Errors
    ///
    /// Returns an error if the page is too short or contains invalid data.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 8 {
            return Err(Error::Serialization(
                "section directory too short".to_string(),
            ));
        }

        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        if count > MAX_SECTIONS {
            return Err(Error::Serialization(format!(
                "section directory has {count} entries, max is {MAX_SECTIONS}"
            )));
        }

        let required = 8 + count * SectionDirectoryEntry::SIZE;
        if data.len() < required {
            return Err(Error::Serialization(format!(
                "section directory data too short: need {required} bytes, got {}",
                data.len()
            )));
        }

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let offset = 8 + i * SectionDirectoryEntry::SIZE;
            let entry = read_entry(&data[offset..offset + SectionDirectoryEntry::SIZE])?;
            entries.push(entry);
        }

        Ok(Self { entries })
    }

    /// Compute CRC-32 of the serialized directory (used in DbHeader).
    #[must_use]
    pub fn checksum(&self) -> u32 {
        let bytes = self.to_bytes();
        crc32fast::hash(&bytes)
    }
}

impl Default for SectionDirectory {
    fn default() -> Self {
        Self::new()
    }
}

// ── Binary serialization for directory entries ──────────────────────

fn write_entry(buf: &mut [u8], entry: &SectionDirectoryEntry) {
    buf[0..4].copy_from_slice(&(entry.section_type as u32).to_le_bytes());
    buf[4] = entry.version;
    buf[5] = entry.flags.to_byte();
    buf[6..8].copy_from_slice(&[0, 0]); // reserved
    buf[8..16].copy_from_slice(&entry.offset.to_le_bytes());
    buf[16..24].copy_from_slice(&entry.length.to_le_bytes());
    buf[24..28].copy_from_slice(&entry.checksum.to_le_bytes());
    buf[28..32].copy_from_slice(&[0, 0, 0, 0]); // reserved
}

fn read_entry(buf: &[u8]) -> Result<SectionDirectoryEntry> {
    let type_val = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let section_type = match type_val {
        1 => SectionType::Catalog,
        2 => SectionType::LpgStore,
        3 => SectionType::RdfStore,
        10 => SectionType::VectorStore,
        11 => SectionType::TextIndex,
        12 => SectionType::RdfRing,
        20 => SectionType::PropertyIndex,
        other => {
            return Err(Error::Serialization(format!(
                "unknown section type: {other}"
            )));
        }
    };

    Ok(SectionDirectoryEntry {
        section_type,
        version: buf[4],
        flags: SectionFlags::from_byte(buf[5]),
        offset: u64::from_le_bytes(buf[8..16].try_into().unwrap()),
        length: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
        checksum: u32::from_le_bytes(buf[24..28].try_into().unwrap()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_directory_round_trip() {
        let dir = SectionDirectory::new();
        let bytes = dir.to_bytes();
        assert_eq!(bytes.len(), DIRECTORY_PAGE_SIZE);

        let dir2 = SectionDirectory::from_bytes(&bytes).unwrap();
        assert!(dir2.is_empty());
    }

    #[test]
    fn single_entry_round_trip() {
        let mut dir = SectionDirectory::new();
        dir.upsert(SectionDirectoryEntry {
            section_type: SectionType::LpgStore,
            version: 1,
            flags: SectionFlags {
                required: true,
                mmap_able: false,
            },
            offset: SECTION_DATA_OFFSET,
            length: 1024,
            checksum: 0xDEADBEEF,
        })
        .unwrap();

        let bytes = dir.to_bytes();
        let dir2 = SectionDirectory::from_bytes(&bytes).unwrap();

        assert_eq!(dir2.len(), 1);
        let entry = dir2.find(SectionType::LpgStore).unwrap();
        assert_eq!(entry.version, 1);
        assert!(entry.flags.required);
        assert!(!entry.flags.mmap_able);
        assert_eq!(entry.offset, SECTION_DATA_OFFSET);
        assert_eq!(entry.length, 1024);
        assert_eq!(entry.checksum, 0xDEADBEEF);
    }

    #[test]
    fn multiple_entries_round_trip() {
        let mut dir = SectionDirectory::new();
        for (i, st) in [
            SectionType::Catalog,
            SectionType::LpgStore,
            SectionType::VectorStore,
        ]
        .iter()
        .enumerate()
        {
            dir.upsert(SectionDirectoryEntry {
                section_type: *st,
                version: 1,
                flags: st.default_flags(),
                offset: SECTION_DATA_OFFSET + (i as u64) * 4096,
                length: 4096,
                checksum: i as u32,
            })
            .unwrap();
        }

        let bytes = dir.to_bytes();
        let dir2 = SectionDirectory::from_bytes(&bytes).unwrap();
        assert_eq!(dir2.len(), 3);

        assert!(dir2.find(SectionType::Catalog).is_some());
        assert!(dir2.find(SectionType::LpgStore).is_some());
        assert!(dir2.find(SectionType::VectorStore).is_some());
        assert!(dir2.find(SectionType::RdfStore).is_none());
    }

    #[test]
    fn upsert_replaces_existing() {
        let mut dir = SectionDirectory::new();
        dir.upsert(SectionDirectoryEntry {
            section_type: SectionType::LpgStore,
            version: 1,
            flags: SectionType::LpgStore.default_flags(),
            offset: SECTION_DATA_OFFSET,
            length: 1024,
            checksum: 100,
        })
        .unwrap();

        dir.upsert(SectionDirectoryEntry {
            section_type: SectionType::LpgStore,
            version: 2,
            flags: SectionType::LpgStore.default_flags(),
            offset: SECTION_DATA_OFFSET + 4096,
            length: 2048,
            checksum: 200,
        })
        .unwrap();

        assert_eq!(dir.len(), 1);
        let entry = dir.find(SectionType::LpgStore).unwrap();
        assert_eq!(entry.version, 2);
        assert_eq!(entry.length, 2048);
        assert_eq!(entry.checksum, 200);
    }

    #[test]
    fn remove_section() {
        let mut dir = SectionDirectory::new();
        dir.upsert(SectionDirectoryEntry {
            section_type: SectionType::Catalog,
            version: 1,
            flags: SectionType::Catalog.default_flags(),
            offset: SECTION_DATA_OFFSET,
            length: 512,
            checksum: 0,
        })
        .unwrap();

        assert!(dir.remove(SectionType::Catalog));
        assert!(dir.is_empty());
        assert!(!dir.remove(SectionType::Catalog));
    }

    #[test]
    fn directory_checksum_deterministic() {
        let mut dir = SectionDirectory::new();
        dir.upsert(SectionDirectoryEntry {
            section_type: SectionType::Catalog,
            version: 1,
            flags: SectionType::Catalog.default_flags(),
            offset: SECTION_DATA_OFFSET,
            length: 512,
            checksum: 42,
        })
        .unwrap();

        let c1 = dir.checksum();
        let c2 = dir.checksum();
        assert_eq!(c1, c2);
        assert_ne!(c1, 0);
    }

    #[test]
    fn page_is_4kib() {
        let dir = SectionDirectory::new();
        assert_eq!(dir.to_bytes().len(), 4096);
    }
}
