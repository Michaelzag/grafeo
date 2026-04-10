//! Section types and traits for the `.grafeo` container format.
//!
//! A `.grafeo` file is a container of typed sections. Each section holds
//! one kind of data (LPG nodes, RDF triples, vector indexes, etc.) and
//! can be independently read, written, checksummed, and mmap'd.
//!
//! The [`Section`] trait is the contract between serializers (grafeo-core)
//! and the container I/O layer (grafeo-storage). Serializers produce opaque
//! bytes; the container writes them to disk without knowing the contents.

use serde::{Deserialize, Serialize};

use crate::utils::error::Result;

// ── Section Type ────────────────────────────────────────────────────

/// Identifies a section type in the container directory.
///
/// Types 1-9 are **data sections** (authoritative, cannot be rebuilt).
/// Types 10-19 are **index sections** (derived, can be rebuilt from data).
/// Types 20+ are reserved for future acceleration structures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
#[non_exhaustive]
pub enum SectionType {
    /// Schema definitions, index metadata, epoch, configuration.
    Catalog = 1,
    /// LPG nodes, edges, properties, named graphs.
    LpgStore = 2,
    /// RDF triples and named graphs.
    RdfStore = 3,

    /// Vector embeddings, HNSW topology, quantization data.
    VectorStore = 10,
    /// BM25 inverted index: term dictionary, postings lists.
    TextIndex = 11,
    /// RDF Ring index: wavelet trees, succinct permutations.
    RdfRing = 12,
    /// Property hash/btree indexes.
    PropertyIndex = 20,
}

impl SectionType {
    /// Whether this section type holds authoritative data (not rebuildable).
    #[must_use]
    pub const fn is_data_section(self) -> bool {
        (self as u32) < 10
    }

    /// Whether this section type holds a derived index (rebuildable from data).
    #[must_use]
    pub const fn is_index_section(self) -> bool {
        (self as u32) >= 10
    }
}

// ── Section Flags ───────────────────────────────────────────────────

/// Flags for a section entry in the container directory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionFlags {
    /// Bit 0: section is required (older binaries must refuse to open if unknown).
    /// When false, unknown section types can be safely skipped.
    pub required: bool,
    /// Bit 1: section data can be mmap'd for zero-copy access.
    pub mmap_able: bool,
}

impl SectionFlags {
    /// Pack flags into a single byte for on-disk storage.
    #[must_use]
    pub const fn to_byte(self) -> u8 {
        let mut flags = 0u8;
        if self.required {
            flags |= 0x01;
        }
        if self.mmap_able {
            flags |= 0x02;
        }
        flags
    }

    /// Unpack flags from a single byte.
    #[must_use]
    pub const fn from_byte(byte: u8) -> Self {
        Self {
            required: byte & 0x01 != 0,
            mmap_able: byte & 0x02 != 0,
        }
    }
}

impl SectionType {
    /// Default flags for this section type.
    #[must_use]
    pub const fn default_flags(self) -> SectionFlags {
        match self {
            Self::Catalog | Self::LpgStore => SectionFlags {
                required: true,
                mmap_able: false,
            },
            Self::RdfStore => SectionFlags {
                required: false,
                mmap_able: false,
            },
            Self::VectorStore | Self::TextIndex | Self::RdfRing | Self::PropertyIndex => {
                SectionFlags {
                    required: false,
                    mmap_able: true,
                }
            }
        }
    }
}

// ── Section Directory Entry ─────────────────────────────────────────

/// A single entry in the container's section directory.
///
/// Fixed 32-byte layout for on-disk storage:
///
/// | Offset | Size | Field |
/// |--------|------|-------|
/// | 0 | 4 | `section_type` (u32 LE) |
/// | 4 | 1 | `version` (u8) |
/// | 5 | 1 | `flags` (packed byte) |
/// | 6 | 2 | reserved (zero) |
/// | 8 | 8 | `offset` (u64 LE, byte offset from file start) |
/// | 16 | 8 | `length` (u64 LE, byte length of section data) |
/// | 24 | 4 | `checksum` (u32 LE, CRC-32 of section data) |
/// | 28 | 4 | reserved (zero) |
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionDirectoryEntry {
    /// Which section type this entry describes.
    pub section_type: SectionType,
    /// Per-section format version (allows independent evolution).
    pub version: u8,
    /// Section flags (required, mmap-able).
    pub flags: SectionFlags,
    /// Byte offset from file start where section data begins.
    pub offset: u64,
    /// Byte length of the section data.
    pub length: u64,
    /// CRC-32 checksum of the section data.
    pub checksum: u32,
}

impl SectionDirectoryEntry {
    /// Size of a directory entry on disk (fixed 32 bytes).
    pub const SIZE: usize = 32;
}

// ── Section Trait ───────────────────────────────────────────────────

/// A serializable section for the `.grafeo` container.
///
/// Implemented in `grafeo-core` for each data model (LPG, RDF) and index
/// type (Vector, Text, Ring). The container I/O layer in `grafeo-storage`
/// calls `serialize()` and `deserialize()` without knowing the section internals.
///
/// The unified flush model uses this trait: the engine iterates all sections,
/// serializes dirty ones, and passes the bytes to the container writer.
pub trait Section: Send + Sync {
    /// The section type identifier.
    fn section_type(&self) -> SectionType;

    /// Per-section format version.
    fn version(&self) -> u8 {
        1
    }

    /// Serialize section contents to bytes.
    ///
    /// Called by the flush path (checkpoint, eviction, explicit CHECKPOINT).
    /// The returned bytes are opaque to the container writer.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails (e.g., encoding error).
    fn serialize(&self) -> Result<Vec<u8>>;

    /// Populate section contents from bytes.
    ///
    /// Called during recovery (loading from container) or reload (mmap to RAM).
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization fails (e.g., corrupt data, version mismatch).
    fn deserialize(&mut self, data: &[u8]) -> Result<()>;

    /// Whether this section has been modified since the last flush.
    fn is_dirty(&self) -> bool;

    /// Mark the section as clean after a successful flush.
    fn mark_clean(&self);

    /// Estimated memory usage of this section in bytes.
    fn memory_usage(&self) -> usize;
}

// ── Tier Override ───────────────────────────────────────────────────

/// Controls whether a section stays in RAM, on disk, or is auto-managed.
///
/// The default (`Auto`) lets the [`BufferManager`](crate::memory::buffer::BufferManager)
/// decide based on memory pressure. Power users can pin a section to a
/// specific tier for predictable performance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TierOverride {
    /// Memory-first, spill to disk when budget exceeded (default).
    #[default]
    Auto,
    /// Always keep in RAM. Fail with error if insufficient memory.
    ForceRam,
    /// Always use disk (mmap). Minimal RAM footprint.
    ForceDisk,
}

/// Per-section memory configuration.
///
/// Allows power users to cap individual sections or pin them to a tier.
/// Most users leave this at default (all sections auto-managed within the
/// global memory budget).
#[derive(Debug, Clone)]
pub struct SectionMemoryConfig {
    /// Hard cap on this section's RAM usage (bytes).
    /// `None` means the section participates in the global budget with no
    /// per-section cap. The BufferManager decides when to spill.
    pub max_ram: Option<usize>,
    /// Storage tier override.
    pub tier: TierOverride,
}

impl Default for SectionMemoryConfig {
    fn default() -> Self {
        Self {
            max_ram: None,
            tier: TierOverride::Auto,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_type_classification() {
        assert!(SectionType::Catalog.is_data_section());
        assert!(SectionType::LpgStore.is_data_section());
        assert!(SectionType::RdfStore.is_data_section());
        assert!(!SectionType::VectorStore.is_data_section());

        assert!(!SectionType::Catalog.is_index_section());
        assert!(SectionType::VectorStore.is_index_section());
        assert!(SectionType::TextIndex.is_index_section());
        assert!(SectionType::RdfRing.is_index_section());
        assert!(SectionType::PropertyIndex.is_index_section());
    }

    #[test]
    fn section_flags_roundtrip() {
        let flags = SectionFlags {
            required: true,
            mmap_able: false,
        };
        assert_eq!(flags.to_byte(), 0x01);
        assert_eq!(SectionFlags::from_byte(0x01), flags);

        let flags = SectionFlags {
            required: false,
            mmap_able: true,
        };
        assert_eq!(flags.to_byte(), 0x02);
        assert_eq!(SectionFlags::from_byte(0x02), flags);

        let flags = SectionFlags {
            required: true,
            mmap_able: true,
        };
        assert_eq!(flags.to_byte(), 0x03);
        assert_eq!(SectionFlags::from_byte(0x03), flags);

        let empty = SectionFlags::default();
        assert_eq!(empty.to_byte(), 0x00);
        assert_eq!(SectionFlags::from_byte(0x00), empty);
    }

    #[test]
    fn default_flags_by_type() {
        let catalog = SectionType::Catalog.default_flags();
        assert!(catalog.required);
        assert!(!catalog.mmap_able);

        let vector = SectionType::VectorStore.default_flags();
        assert!(!vector.required);
        assert!(vector.mmap_able);

        let rdf = SectionType::RdfStore.default_flags();
        assert!(!rdf.required);
        assert!(!rdf.mmap_able);
    }

    #[test]
    fn directory_entry_size() {
        assert_eq!(SectionDirectoryEntry::SIZE, 32);
    }
}
