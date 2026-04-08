//! RDF section serializer for the `.grafeo` container format.
//!
//! Implements the [`Section`] trait for RDF triple data (triples, named graphs).
//! Produces bincode-encoded bytes for the `RDF_STORE` section.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use grafeo_common::storage::section::{Section, SectionType};
use grafeo_common::utils::error::{Error, Result};

use crate::graph::rdf::{RdfStore, Term, Triple};

/// Current RDF section format version.
const RDF_SECTION_VERSION: u8 = 1;

// ── Snapshot types ──────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct RdfSnapshot {
    version: u8,
    triples: Vec<SnapshotTriple>,
    named_graphs: Vec<RdfNamedGraphSnapshot>,
}

#[derive(Serialize, Deserialize)]
struct SnapshotTriple {
    subject: String,
    predicate: String,
    object: String,
}

#[derive(Serialize, Deserialize)]
struct RdfNamedGraphSnapshot {
    name: String,
    triples: Vec<SnapshotTriple>,
}

// ── Collection helpers ──────────────────────────────────────────────

fn collect_triples(store: &RdfStore) -> Vec<SnapshotTriple> {
    store
        .triples()
        .into_iter()
        .map(|t| SnapshotTriple {
            subject: t.subject().to_string(),
            predicate: t.predicate().to_string(),
            object: t.object().to_string(),
        })
        .collect()
}

fn populate_store(store: &RdfStore, triples: &[SnapshotTriple]) {
    for triple in triples {
        if let (Some(s), Some(p), Some(o)) = (
            Term::from_ntriples(&triple.subject),
            Term::from_ntriples(&triple.predicate),
            Term::from_ntriples(&triple.object),
        ) {
            store.insert(Triple::new(s, p, o));
        }
    }
}

// ── Section implementation ──────────────────────────────────────────

/// RDF store section for the `.grafeo` container.
pub struct RdfStoreSection {
    store: Arc<RdfStore>,
    dirty: AtomicBool,
}

impl RdfStoreSection {
    /// Create a new RDF section wrapping the given store.
    pub fn new(store: Arc<RdfStore>) -> Self {
        Self {
            store,
            dirty: AtomicBool::new(false),
        }
    }

    /// Mark this section as dirty.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    /// Access the underlying store.
    #[must_use]
    pub fn store(&self) -> &Arc<RdfStore> {
        &self.store
    }
}

impl Section for RdfStoreSection {
    fn section_type(&self) -> SectionType {
        SectionType::RdfStore
    }

    fn version(&self) -> u8 {
        RDF_SECTION_VERSION
    }

    fn serialize(&self) -> Result<Vec<u8>> {
        let triples = collect_triples(&self.store);

        let named_graphs: Vec<RdfNamedGraphSnapshot> = self
            .store
            .graph_names()
            .into_iter()
            .filter_map(|name| {
                self.store.graph(&name).map(|graph| RdfNamedGraphSnapshot {
                    name,
                    triples: collect_triples(&graph),
                })
            })
            .collect();

        let snapshot = RdfSnapshot {
            version: RDF_SECTION_VERSION,
            triples,
            named_graphs,
        };

        let config = bincode::config::standard();
        bincode::serde::encode_to_vec(&snapshot, config)
            .map_err(|e| Error::Internal(format!("RDF section serialization failed: {e}")))
    }

    fn deserialize(&mut self, data: &[u8]) -> Result<()> {
        let config = bincode::config::standard();
        let (snapshot, _): (RdfSnapshot, _) = bincode::serde::decode_from_slice(data, config)
            .map_err(|e| {
                Error::Serialization(format!("RDF section deserialization failed: {e}"))
            })?;

        populate_store(&self.store, &snapshot.triples);

        for graph in &snapshot.named_graphs {
            self.store.create_graph(&graph.name);
            if let Some(graph_store) = self.store.graph(&graph.name) {
                populate_store(&graph_store, &graph.triples);
            }
        }

        Ok(())
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    fn mark_clean(&self) {
        self.dirty.store(false, Ordering::Release);
    }

    fn memory_usage(&self) -> usize {
        // RdfStore doesn't expose a memory_breakdown yet; estimate from triple count
        self.store.len() * 200 // ~200 bytes per triple (rough estimate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rdf_section_round_trip() {
        let store = Arc::new(RdfStore::new());
        store.insert(Triple::new(
            Term::iri("http://example.org/alix"),
            Term::iri("http://xmlns.com/foaf/0.1/name"),
            Term::literal("Alix"),
        ));
        store.insert(Triple::new(
            Term::iri("http://example.org/gus"),
            Term::iri("http://xmlns.com/foaf/0.1/name"),
            Term::literal("Gus"),
        ));

        let section = RdfStoreSection::new(Arc::clone(&store));
        let bytes = section.serialize().expect("serialize should succeed");
        assert!(!bytes.is_empty());

        let store2 = Arc::new(RdfStore::new());
        let mut section2 = RdfStoreSection::new(store2);
        section2
            .deserialize(&bytes)
            .expect("deserialize should succeed");

        assert_eq!(section2.store().len(), 2);
    }

    #[test]
    fn rdf_section_type() {
        let store = Arc::new(RdfStore::new());
        let section = RdfStoreSection::new(store);
        assert_eq!(section.section_type(), SectionType::RdfStore);
    }
}
