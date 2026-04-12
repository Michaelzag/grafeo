//! Ring Index section for `.grafeo` container persistence.
//!
//! Serializes and deserializes the [`TripleRing`] via the [`Section`] trait,
//! enabling the Ring to survive database restarts without rebuilding from
//! triples. Uses bincode for encoding.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use grafeo_common::storage::section::{Section, SectionType};
use grafeo_common::utils::error::{Error, Result};

use crate::graph::rdf::RdfStore;

const RING_SECTION_VERSION: u8 = 1;

/// Section implementation for the RDF Ring Index.
///
/// Wraps an `Arc<RdfStore>` and serializes/deserializes the Ring via
/// `TripleRing::save_to_bytes()`/`load_from_bytes()`.
pub struct RdfRingSection {
    store: Arc<RdfStore>,
    dirty: AtomicBool,
}

impl RdfRingSection {
    /// Creates a new Ring section backed by the given RDF store.
    #[must_use]
    pub fn new(store: Arc<RdfStore>) -> Self {
        Self {
            store,
            dirty: AtomicBool::new(false),
        }
    }

    /// Marks the section as dirty (Ring was rebuilt or invalidated).
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }
}

impl Section for RdfRingSection {
    fn section_type(&self) -> SectionType {
        SectionType::RdfRing
    }

    fn version(&self) -> u8 {
        RING_SECTION_VERSION
    }

    fn serialize(&self) -> Result<Vec<u8>> {
        match self.store.ring() {
            Some(ring) => ring
                .save_to_bytes()
                .map_err(|e| Error::Serialization(e.to_string())),
            None => Ok(Vec::new()),
        }
    }

    fn deserialize(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        let ring = super::TripleRing::load_from_bytes(data)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        self.store.set_ring(ring);
        Ok(())
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    fn mark_clean(&self) {
        self.dirty.store(false, Ordering::Release);
    }

    fn memory_usage(&self) -> usize {
        self.store.ring().map_or(0, |r| r.size_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::rdf::{Term, Triple};

    fn test_store() -> Arc<RdfStore> {
        let store = Arc::new(RdfStore::new());
        store.bulk_load(vec![
            Triple::new(
                Term::iri("http://ex.org/alix"),
                Term::iri("http://xmlns.com/foaf/0.1/name"),
                Term::literal("Alix"),
            ),
            Triple::new(
                Term::iri("http://ex.org/gus"),
                Term::iri("http://xmlns.com/foaf/0.1/name"),
                Term::literal("Gus"),
            ),
            Triple::new(
                Term::iri("http://ex.org/alix"),
                Term::iri("http://xmlns.com/foaf/0.1/knows"),
                Term::iri("http://ex.org/gus"),
            ),
        ]);
        store
    }

    #[test]
    fn section_type_is_rdf_ring() {
        let store = test_store();
        let section = RdfRingSection::new(store);
        assert_eq!(section.section_type(), SectionType::RdfRing);
        assert_eq!(section.version(), 1);
    }

    #[test]
    fn section_dirty_tracking() {
        let store = test_store();
        let section = RdfRingSection::new(store);
        assert!(!section.is_dirty());
        section.mark_dirty();
        assert!(section.is_dirty());
        section.mark_clean();
        assert!(!section.is_dirty());
    }

    #[test]
    fn section_serialize_empty() {
        let store = Arc::new(RdfStore::new());
        let section = RdfRingSection::new(store);
        let bytes = section.serialize().unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn section_roundtrip() {
        let store = test_store();
        let section = RdfRingSection::new(Arc::clone(&store));

        // Serialize
        let bytes = section.serialize().unwrap();
        assert!(!bytes.is_empty());

        // Create a fresh store and deserialize into it
        let store2 = Arc::new(RdfStore::new());
        let mut section2 = RdfRingSection::new(Arc::clone(&store2));
        section2.deserialize(&bytes).unwrap();

        // The loaded ring should have the same triple count
        let ring = store2.ring().expect("ring should be loaded");
        assert_eq!(ring.len(), 3);

        // Verify count operations work
        use crate::graph::rdf::TriplePattern;
        let name_pattern = TriplePattern {
            subject: None,
            predicate: Some(Term::iri("http://xmlns.com/foaf/0.1/name")),
            object: None,
        };
        assert_eq!(ring.count(&name_pattern), 2);
    }

    #[test]
    fn section_memory_usage() {
        let store = test_store();
        let section = RdfRingSection::new(store);
        assert!(section.memory_usage() > 0);
    }
}
