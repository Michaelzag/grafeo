//! Adapts storage sections into [`MemoryConsumer`]s for BufferManager integration.
//!
//! Each section (LPG, RDF, Vector, Text, Catalog) is registered with the
//! [`BufferManager`] so that memory tracking and pressure awareness include
//! section memory. This enables accurate `memory_usage()` reporting and
//! lays the groundwork for automatic spilling when tiered storage is added.

use std::sync::Arc;

use grafeo_common::memory::buffer::{MemoryConsumer, MemoryRegion, SpillError, priorities};
use grafeo_common::storage::Section;

/// Wraps a [`Section`] as a [`MemoryConsumer`] for the BufferManager.
///
/// Data sections (Catalog, LPG, RDF) use [`GRAPH_STORAGE`](priorities::GRAPH_STORAGE)
/// priority (evict last). Index sections (Vector, Text, RdfRing, PropertyIndex)
/// use [`INDEX_BUFFERS`](priorities::INDEX_BUFFERS) priority (evict before data).
///
/// Currently, `evict()` returns 0 because sections cannot release memory
/// without a full checkpoint + mmap cycle. The [`can_spill`](MemoryConsumer::can_spill)
/// method returns `true` for mmap-able index sections, signaling that future
/// tiered storage support will enable actual spilling.
pub struct SectionConsumer {
    name: String,
    section: Arc<dyn Section>,
    priority: u8,
    region: MemoryRegion,
    mmap_able: bool,
}

impl SectionConsumer {
    /// Creates a consumer for the given section.
    ///
    /// Priority and region are assigned based on the section type:
    /// - Data sections (types 1-9): `GRAPH_STORAGE` priority, `GraphStorage` region
    /// - Index sections (types 10+): `INDEX_BUFFERS` priority, `IndexBuffers` region
    pub fn new(section: Arc<dyn Section>) -> Self {
        let section_type = section.section_type();
        let is_data = section_type.is_data_section();
        let flags = section_type.default_flags();

        Self {
            name: format!("section:{section_type:?}"),
            section,
            priority: if is_data {
                priorities::GRAPH_STORAGE
            } else {
                priorities::INDEX_BUFFERS
            },
            region: if is_data {
                MemoryRegion::GraphStorage
            } else {
                MemoryRegion::IndexBuffers
            },
            mmap_able: flags.mmap_able,
        }
    }
}

impl MemoryConsumer for SectionConsumer {
    fn name(&self) -> &str {
        &self.name
    }

    fn memory_usage(&self) -> usize {
        self.section.memory_usage()
    }

    fn eviction_priority(&self) -> u8 {
        self.priority
    }

    fn region(&self) -> MemoryRegion {
        self.region
    }

    fn evict(&self, _target_bytes: usize) -> usize {
        // Sections cannot evict in-place. Freeing section memory requires
        // a checkpoint (serialize + write to container) followed by mmap.
        // The engine handles this at a higher level when pressure is detected.
        0
    }

    fn can_spill(&self) -> bool {
        // Index sections with mmap support can be spilled to the container
        // and served via memory-mapped I/O. Data sections require full
        // deserialization and cannot be mmap'd (yet).
        self.mmap_able
    }

    fn spill(&self, _target_bytes: usize) -> Result<usize, SpillError> {
        if !self.mmap_able {
            return Err(SpillError::NotSupported);
        }
        // Actual spill implementation will be added with tiered storage:
        // 1. Serialize section via Section::serialize()
        // 2. Write to container via GrafeoFileManager::write_sections()
        // 3. Mmap the section via GrafeoFileManager::mmap_section()
        // 4. Switch section to mmap-backed read mode
        // 5. Drop in-memory data, return freed bytes
        Err(SpillError::NotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grafeo_common::storage::section::SectionType;
    use grafeo_common::utils::error::Result;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Minimal Section implementation for testing.
    struct FakeSection {
        section_type: SectionType,
        usage: usize,
        dirty: AtomicBool,
    }

    impl FakeSection {
        fn new(section_type: SectionType, usage: usize) -> Self {
            Self {
                section_type,
                usage,
                dirty: AtomicBool::new(false),
            }
        }
    }

    impl Section for FakeSection {
        fn section_type(&self) -> SectionType {
            self.section_type
        }
        fn serialize(&self) -> Result<Vec<u8>> {
            Ok(vec![0; self.usage])
        }
        fn deserialize(&mut self, _data: &[u8]) -> Result<()> {
            Ok(())
        }
        fn is_dirty(&self) -> bool {
            self.dirty.load(Ordering::Relaxed)
        }
        fn mark_clean(&self) {
            self.dirty.store(false, Ordering::Relaxed);
        }
        fn memory_usage(&self) -> usize {
            self.usage
        }
    }

    #[test]
    fn data_section_consumer_properties() {
        let section = Arc::new(FakeSection::new(SectionType::LpgStore, 1024));
        let consumer = SectionConsumer::new(section);

        assert_eq!(consumer.name(), "section:LpgStore");
        assert_eq!(consumer.memory_usage(), 1024);
        assert_eq!(consumer.eviction_priority(), priorities::GRAPH_STORAGE);
        assert_eq!(consumer.region(), MemoryRegion::GraphStorage);
        assert!(!consumer.can_spill());
    }

    #[test]
    fn index_section_consumer_properties() {
        let section = Arc::new(FakeSection::new(SectionType::VectorStore, 4096));
        let consumer = SectionConsumer::new(section);

        assert_eq!(consumer.name(), "section:VectorStore");
        assert_eq!(consumer.memory_usage(), 4096);
        assert_eq!(consumer.eviction_priority(), priorities::INDEX_BUFFERS);
        assert_eq!(consumer.region(), MemoryRegion::IndexBuffers);
        assert!(consumer.can_spill());
    }

    #[test]
    fn evict_returns_zero() {
        let section = Arc::new(FakeSection::new(SectionType::TextIndex, 8192));
        let consumer = SectionConsumer::new(section);

        // Sections can't evict in-place
        assert_eq!(consumer.evict(4096), 0);
        // Memory is unchanged
        assert_eq!(consumer.memory_usage(), 8192);
    }

    #[test]
    fn spill_returns_not_supported() {
        let section = Arc::new(FakeSection::new(SectionType::VectorStore, 4096));
        let consumer = SectionConsumer::new(section);

        let result = consumer.spill(2048);
        assert!(result.is_err());
    }

    #[test]
    fn catalog_section_is_data() {
        let section = Arc::new(FakeSection::new(SectionType::Catalog, 256));
        let consumer = SectionConsumer::new(section);

        assert_eq!(consumer.eviction_priority(), priorities::GRAPH_STORAGE);
        assert!(!consumer.can_spill());
    }

    #[test]
    fn rdf_ring_section_is_index() {
        let section = Arc::new(FakeSection::new(SectionType::RdfRing, 2048));
        let consumer = SectionConsumer::new(section);

        assert_eq!(consumer.eviction_priority(), priorities::INDEX_BUFFERS);
        assert!(consumer.can_spill());
    }
}
