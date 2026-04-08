//! LPG section serializer for the `.grafeo` container format.
//!
//! Implements the [`Section`] trait for LPG graph data (nodes, edges,
//! properties, named graphs). Produces bincode-encoded bytes that the
//! container writer stores as the `LPG_STORE` section.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use grafeo_common::storage::section::{Section, SectionType};
use grafeo_common::types::{EdgeId, EpochId, NodeId, Value};
use grafeo_common::utils::error::{Error, Result};

use crate::graph::lpg::LpgStore;

/// Current LPG section format version.
const LPG_SECTION_VERSION: u8 = 1;

// ── Snapshot types (bincode-serializable) ───────────────────────────

#[derive(Serialize, Deserialize)]
struct LpgSnapshot {
    version: u8,
    nodes: Vec<SnapshotNode>,
    edges: Vec<SnapshotEdge>,
    named_graphs: Vec<NamedGraphSnapshot>,
    epoch: u64,
}

#[derive(Serialize, Deserialize)]
struct SnapshotNode {
    id: NodeId,
    labels: Vec<String>,
    properties: Vec<(String, Vec<(EpochId, Value)>)>,
}

#[derive(Serialize, Deserialize)]
struct SnapshotEdge {
    id: EdgeId,
    src: NodeId,
    dst: NodeId,
    edge_type: String,
    properties: Vec<(String, Vec<(EpochId, Value)>)>,
}

#[derive(Serialize, Deserialize)]
struct NamedGraphSnapshot {
    name: String,
    nodes: Vec<SnapshotNode>,
    edges: Vec<SnapshotEdge>,
}

// ── Collection helpers ──────────────────────────────────────────────

fn collect_nodes(store: &LpgStore) -> Vec<SnapshotNode> {
    let mut nodes: Vec<SnapshotNode> = store
        .all_nodes()
        .map(|n| {
            #[cfg(feature = "temporal")]
            let mut properties: Vec<(String, Vec<(EpochId, Value)>)> = store
                .node_property_history(n.id)
                .into_iter()
                .map(|(k, entries)| (k.to_string(), entries))
                .collect();

            #[cfg(not(feature = "temporal"))]
            let mut properties: Vec<(String, Vec<(EpochId, Value)>)> = n
                .properties
                .into_iter()
                .map(|(k, v)| (k.to_string(), vec![(EpochId::new(0), v)]))
                .collect();

            properties.sort_by(|(a, _), (b, _)| a.cmp(b));

            let mut labels: Vec<String> = n.labels.iter().map(|l| l.to_string()).collect();
            labels.sort();

            SnapshotNode {
                id: n.id,
                labels,
                properties,
            }
        })
        .collect();
    nodes.sort_by_key(|n| n.id);
    nodes
}

fn collect_edges(store: &LpgStore) -> Vec<SnapshotEdge> {
    let mut edges: Vec<SnapshotEdge> = store
        .all_edges()
        .map(|e| {
            #[cfg(feature = "temporal")]
            let mut properties: Vec<(String, Vec<(EpochId, Value)>)> = store
                .edge_property_history(e.id)
                .into_iter()
                .map(|(k, entries)| (k.to_string(), entries))
                .collect();

            #[cfg(not(feature = "temporal"))]
            let mut properties: Vec<(String, Vec<(EpochId, Value)>)> = e
                .properties
                .into_iter()
                .map(|(k, v)| (k.to_string(), vec![(EpochId::new(0), v)]))
                .collect();

            properties.sort_by(|(a, _), (b, _)| a.cmp(b));

            SnapshotEdge {
                id: e.id,
                src: e.src,
                dst: e.dst,
                edge_type: e.edge_type.to_string(),
                properties,
            }
        })
        .collect();
    edges.sort_by_key(|e| e.id);
    edges
}

fn populate_store(store: &LpgStore, nodes: &[SnapshotNode], edges: &[SnapshotEdge]) -> Result<()> {
    for node in nodes {
        let label_refs: Vec<&str> = node.labels.iter().map(|s| s.as_str()).collect();
        store.create_node_with_id(node.id, &label_refs)?;
        for (key, entries) in &node.properties {
            #[cfg(feature = "temporal")]
            for (epoch, value) in entries {
                store.set_node_property_at_epoch(node.id, key, value.clone(), *epoch);
            }
            #[cfg(not(feature = "temporal"))]
            if let Some((_, value)) = entries.last() {
                store.set_node_property(node.id, key, value.clone());
            }
        }
    }
    for edge in edges {
        store.create_edge_with_id(edge.id, edge.src, edge.dst, &edge.edge_type)?;
        for (key, entries) in &edge.properties {
            #[cfg(feature = "temporal")]
            for (epoch, value) in entries {
                store.set_edge_property_at_epoch(edge.id, key, value.clone(), *epoch);
            }
            #[cfg(not(feature = "temporal"))]
            if let Some((_, value)) = entries.last() {
                store.set_edge_property(edge.id, key, value.clone());
            }
        }
    }
    Ok(())
}

// ── Section implementation ──────────────────────────────────────────

/// LPG store section for the `.grafeo` container.
///
/// Wraps an `Arc<LpgStore>` and implements the [`Section`] trait for
/// serialization/deserialization of LPG graph data.
pub struct LpgStoreSection {
    store: Arc<LpgStore>,
    dirty: AtomicBool,
}

impl LpgStoreSection {
    /// Create a new LPG section wrapping the given store.
    pub fn new(store: Arc<LpgStore>) -> Self {
        Self {
            store,
            dirty: AtomicBool::new(false),
        }
    }

    /// Mark this section as dirty (has unsaved changes).
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    /// Access the underlying store.
    #[must_use]
    pub fn store(&self) -> &Arc<LpgStore> {
        &self.store
    }
}

impl Section for LpgStoreSection {
    fn section_type(&self) -> SectionType {
        SectionType::LpgStore
    }

    fn version(&self) -> u8 {
        LPG_SECTION_VERSION
    }

    fn serialize(&self) -> Result<Vec<u8>> {
        let nodes = collect_nodes(&self.store);
        let edges = collect_edges(&self.store);

        let named_graphs: Vec<NamedGraphSnapshot> = self
            .store
            .graph_names()
            .into_iter()
            .filter_map(|name| {
                self.store
                    .graph(&name)
                    .map(|graph_store| NamedGraphSnapshot {
                        name,
                        nodes: collect_nodes(&graph_store),
                        edges: collect_edges(&graph_store),
                    })
            })
            .collect();

        #[cfg(feature = "temporal")]
        let epoch = self.store.current_epoch().as_u64();
        #[cfg(not(feature = "temporal"))]
        let epoch = 0u64;

        let snapshot = LpgSnapshot {
            version: LPG_SECTION_VERSION,
            nodes,
            edges,
            named_graphs,
            epoch,
        };

        let config = bincode::config::standard();
        bincode::serde::encode_to_vec(&snapshot, config)
            .map_err(|e| Error::Internal(format!("LPG section serialization failed: {e}")))
    }

    fn deserialize(&mut self, data: &[u8]) -> Result<()> {
        let config = bincode::config::standard();
        let (snapshot, _): (LpgSnapshot, _) = bincode::serde::decode_from_slice(data, config)
            .map_err(|e| {
                Error::Serialization(format!("LPG section deserialization failed: {e}"))
            })?;

        populate_store(&self.store, &snapshot.nodes, &snapshot.edges)?;

        #[cfg(feature = "temporal")]
        self.store.sync_epoch(EpochId::new(snapshot.epoch));

        for graph in &snapshot.named_graphs {
            self.store
                .create_graph(&graph.name)
                .map_err(|e| Error::Internal(e.to_string()))?;
            if let Some(graph_store) = self.store.graph(&graph.name) {
                populate_store(&graph_store, &graph.nodes, &graph.edges)?;
                #[cfg(feature = "temporal")]
                graph_store.sync_epoch(EpochId::new(snapshot.epoch));
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
        let (store, indexes, mvcc, string_pool) = self.store.memory_breakdown();
        store.total_bytes + indexes.total_bytes + mvcc.total_bytes + string_pool.total_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lpg_section_round_trip() {
        let store = Arc::new(LpgStore::new().unwrap());
        store.create_node(&["Person"]);
        store.create_node(&["Person"]);
        let n1 = NodeId::new(1);
        let n2 = NodeId::new(2);
        store.set_node_property(n1, "name", Value::String("Alix".into()));
        store.set_node_property(n2, "name", Value::String("Gus".into()));
        store.create_edge(n1, n2, "KNOWS");

        let section = LpgStoreSection::new(Arc::clone(&store));
        let bytes = section.serialize().expect("serialize should succeed");
        assert!(!bytes.is_empty());

        // Deserialize into a fresh store
        let store2 = Arc::new(LpgStore::new().unwrap());
        let mut section2 = LpgStoreSection::new(store2);
        section2
            .deserialize(&bytes)
            .expect("deserialize should succeed");

        assert_eq!(section2.store().node_count(), 2);
        assert_eq!(section2.store().edge_count(), 1);
    }

    #[test]
    fn lpg_section_dirty_tracking() {
        let store = Arc::new(LpgStore::new().unwrap());
        let section = LpgStoreSection::new(store);

        assert!(!section.is_dirty());
        section.mark_dirty();
        assert!(section.is_dirty());
        section.mark_clean();
        assert!(!section.is_dirty());
    }

    #[test]
    fn lpg_section_type() {
        let store = Arc::new(LpgStore::new().unwrap());
        let section = LpgStoreSection::new(store);
        assert_eq!(section.section_type(), SectionType::LpgStore);
        assert_eq!(section.version(), LPG_SECTION_VERSION);
    }
}
