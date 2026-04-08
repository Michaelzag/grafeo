//! Vector Store section serializer for the `.grafeo` container format.
//!
//! Serializes HNSW topology (neighbor graphs) for all vector indexes.
//! Embeddings are not stored here: they live in LPG node properties and
//! are accessed via `VectorAccessor` during search.
//!
//! Persisting the topology eliminates the O(N log N) HNSW rebuild on
//! database open. For 1M vectors this saves 30-60 seconds of startup time.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use grafeo_common::storage::section::{Section, SectionType};
use grafeo_common::types::NodeId;
use grafeo_common::utils::error::{Error, Result};

use super::{DistanceMetric, HnswIndex};

/// Current vector store section format version.
const VECTOR_SECTION_VERSION: u8 = 1;

// ── Snapshot types ──────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct VectorStoreSnapshot {
    version: u8,
    indexes: Vec<IndexSnapshot>,
}

#[derive(Serialize, Deserialize)]
struct IndexSnapshot {
    /// Index key: "label:property"
    key: String,
    /// HNSW configuration
    dimensions: usize,
    metric: DistanceMetric,
    m: usize,
    ef_construction: usize,
    /// Topology
    entry_point: Option<NodeId>,
    max_level: usize,
    /// Node neighbors: Vec<(NodeId, Vec<Vec<NodeId>>)>
    nodes: Vec<(NodeId, Vec<Vec<NodeId>>)>,
}

// ── Section implementation ──────────────────────────────────────────

/// Vector Store section for the `.grafeo` container.
///
/// Wraps a collection of `(key, Arc<HnswIndex>)` pairs and serializes
/// their HNSW topologies for persistence.
pub struct VectorStoreSection {
    /// Vector indexes: (key, index) pairs from LpgStore::vector_index_entries()
    indexes: Vec<(String, Arc<HnswIndex>)>,
    dirty: AtomicBool,
}

impl VectorStoreSection {
    /// Create a new Vector Store section from the current indexes.
    pub fn new(indexes: Vec<(String, Arc<HnswIndex>)>) -> Self {
        Self {
            indexes,
            dirty: AtomicBool::new(false),
        }
    }

    /// Mark this section as dirty.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }
}

impl Section for VectorStoreSection {
    fn section_type(&self) -> SectionType {
        SectionType::VectorStore
    }

    fn version(&self) -> u8 {
        VECTOR_SECTION_VERSION
    }

    fn serialize(&self) -> Result<Vec<u8>> {
        let indexes: Vec<IndexSnapshot> = self
            .indexes
            .iter()
            .map(|(key, index)| {
                let config = index.config();
                let (entry_point, max_level, nodes) = index.snapshot_topology();

                IndexSnapshot {
                    key: key.clone(),
                    dimensions: config.dimensions,
                    metric: config.metric,
                    m: config.m,
                    ef_construction: config.ef_construction,
                    entry_point,
                    max_level,
                    nodes,
                }
            })
            .collect();

        let snapshot = VectorStoreSnapshot {
            version: VECTOR_SECTION_VERSION,
            indexes,
        };

        let config = bincode::config::standard();
        bincode::serde::encode_to_vec(&snapshot, config)
            .map_err(|e| Error::Internal(format!("Vector Store section serialization failed: {e}")))
    }

    fn deserialize(&mut self, data: &[u8]) -> Result<()> {
        let config = bincode::config::standard();
        let (snapshot, _): (VectorStoreSnapshot, _) =
            bincode::serde::decode_from_slice(data, config).map_err(|e| {
                Error::Serialization(format!("Vector Store section deserialization failed: {e}"))
            })?;

        // Restore topology into existing indexes (matched by key)
        for idx_snap in &snapshot.indexes {
            if let Some((_, index)) = self.indexes.iter().find(|(k, _)| k == &idx_snap.key) {
                index.restore_topology(
                    idx_snap.entry_point,
                    idx_snap.max_level,
                    idx_snap.nodes.clone(),
                );
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
        self.indexes
            .iter()
            .map(|(_, idx)| idx.heap_memory_bytes())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::vector::HnswConfig;

    fn make_test_index() -> (String, Arc<HnswIndex>) {
        let config = HnswConfig::new(4, DistanceMetric::Cosine);
        let index = Arc::new(HnswIndex::new(config));

        // Manually set up a small topology via snapshot/restore
        let nodes = vec![
            (NodeId::new(1), vec![vec![NodeId::new(2), NodeId::new(3)]]),
            (NodeId::new(2), vec![vec![NodeId::new(1), NodeId::new(3)]]),
            (NodeId::new(3), vec![vec![NodeId::new(1), NodeId::new(2)]]),
        ];
        index.restore_topology(Some(NodeId::new(1)), 0, nodes);

        ("Item:embedding".to_string(), index)
    }

    #[test]
    fn vector_section_round_trip() {
        let (key, index) = make_test_index();
        let section = VectorStoreSection::new(vec![(key.clone(), Arc::clone(&index))]);

        let bytes = section.serialize().expect("serialize should succeed");
        assert!(!bytes.is_empty());

        // Create a fresh index with same config to restore into
        let config = index.config().clone();
        let fresh_index = Arc::new(HnswIndex::new(config));
        let mut section2 = VectorStoreSection::new(vec![(key, fresh_index.clone())]);
        section2
            .deserialize(&bytes)
            .expect("deserialize should succeed");

        assert_eq!(fresh_index.len(), 3);
        let (ep, ml, nodes) = fresh_index.snapshot_topology();
        assert_eq!(ep, Some(NodeId::new(1)));
        assert_eq!(ml, 0);
        assert_eq!(nodes.len(), 3);
    }

    #[test]
    fn vector_section_empty() {
        let section = VectorStoreSection::new(vec![]);
        let bytes = section.serialize().expect("serialize should succeed");

        let mut section2 = VectorStoreSection::new(vec![]);
        section2
            .deserialize(&bytes)
            .expect("deserialize should succeed");
    }

    #[test]
    fn vector_section_type() {
        let section = VectorStoreSection::new(vec![]);
        assert_eq!(section.section_type(), SectionType::VectorStore);
        assert_eq!(section.version(), VECTOR_SECTION_VERSION);
    }

    #[test]
    fn vector_section_dirty_tracking() {
        let section = VectorStoreSection::new(vec![]);
        assert!(!section.is_dirty());
        section.mark_dirty();
        assert!(section.is_dirty());
        section.mark_clean();
        assert!(!section.is_dirty());
    }
}
