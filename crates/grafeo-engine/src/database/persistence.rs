//! Persistence, snapshots, and data export for GrafeoDB.

#[cfg(feature = "wal")]
use std::path::Path;

use grafeo_common::types::{EdgeId, NodeId, Value};
use grafeo_common::utils::error::{Error, Result};
use hashbrown::HashSet;

use crate::config::Config;

#[cfg(feature = "wal")]
use grafeo_adapters::storage::wal::WalRecord;

/// Binary snapshot format v1 (no named graphs).
#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    version: u8,
    nodes: Vec<SnapshotNode>,
    edges: Vec<SnapshotEdge>,
}

/// Binary snapshot format v2 (with named graphs).
#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotV2 {
    version: u8,
    nodes: Vec<SnapshotNode>,
    edges: Vec<SnapshotEdge>,
    named_graphs: Vec<NamedGraphSnapshot>,
}

/// A named graph partition within a v2 snapshot.
#[derive(serde::Serialize, serde::Deserialize)]
struct NamedGraphSnapshot {
    name: String,
    nodes: Vec<SnapshotNode>,
    edges: Vec<SnapshotEdge>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotNode {
    id: NodeId,
    labels: Vec<String>,
    properties: Vec<(String, Value)>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotEdge {
    id: EdgeId,
    src: NodeId,
    dst: NodeId,
    edge_type: String,
    properties: Vec<(String, Value)>,
}

/// Collects all nodes from a store into snapshot format.
fn collect_snapshot_nodes(store: &grafeo_core::graph::lpg::LpgStore) -> Vec<SnapshotNode> {
    store
        .all_nodes()
        .map(|n| SnapshotNode {
            id: n.id,
            labels: n.labels.iter().map(|l| l.to_string()).collect(),
            properties: n
                .properties
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        })
        .collect()
}

/// Collects all edges from a store into snapshot format.
fn collect_snapshot_edges(store: &grafeo_core::graph::lpg::LpgStore) -> Vec<SnapshotEdge> {
    store
        .all_edges()
        .map(|e| SnapshotEdge {
            id: e.id,
            src: e.src,
            dst: e.dst,
            edge_type: e.edge_type.to_string(),
            properties: e
                .properties
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        })
        .collect()
}

/// Populates a store from snapshot node/edge data.
fn populate_store_from_snapshot(
    store: &grafeo_core::graph::lpg::LpgStore,
    nodes: Vec<SnapshotNode>,
    edges: Vec<SnapshotEdge>,
) -> Result<()> {
    for node in nodes {
        let label_refs: Vec<&str> = node.labels.iter().map(|s| s.as_str()).collect();
        store.create_node_with_id(node.id, &label_refs)?;
        for (key, value) in node.properties {
            store.set_node_property(node.id, &key, value);
        }
    }
    for edge in edges {
        store.create_edge_with_id(edge.id, edge.src, edge.dst, &edge.edge_type)?;
        for (key, value) in edge.properties {
            store.set_edge_property(edge.id, &key, value);
        }
    }
    Ok(())
}

/// Validates snapshot nodes/edges for duplicates and dangling references.
fn validate_snapshot_data(nodes: &[SnapshotNode], edges: &[SnapshotEdge]) -> Result<()> {
    let mut node_ids = HashSet::with_capacity(nodes.len());
    for node in nodes {
        if !node_ids.insert(node.id) {
            return Err(Error::Internal(format!(
                "snapshot contains duplicate node ID {}",
                node.id
            )));
        }
    }
    let mut edge_ids = HashSet::with_capacity(edges.len());
    for edge in edges {
        if !edge_ids.insert(edge.id) {
            return Err(Error::Internal(format!(
                "snapshot contains duplicate edge ID {}",
                edge.id
            )));
        }
        if !node_ids.contains(&edge.src) {
            return Err(Error::Internal(format!(
                "snapshot edge {} references non-existent source node {}",
                edge.id, edge.src
            )));
        }
        if !node_ids.contains(&edge.dst) {
            return Err(Error::Internal(format!(
                "snapshot edge {} references non-existent destination node {}",
                edge.id, edge.dst
            )));
        }
    }
    Ok(())
}

impl super::GrafeoDB {
    // =========================================================================
    // ADMIN API: Persistence Control
    // =========================================================================

    /// Saves the database to a file path.
    ///
    /// - If in-memory: creates a new persistent database at path
    /// - If file-backed: creates a copy at the new path
    ///
    /// The original database remains unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if the save operation fails.
    ///
    /// Requires the `wal` feature for persistence support.
    #[cfg(feature = "wal")]
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        // Create target database with WAL enabled
        let target_config = Config::persistent(path);
        let target = Self::with_config(target_config)?;

        // Copy all nodes using WAL-enabled methods
        for node in self.store.all_nodes() {
            let label_refs: Vec<&str> = node.labels.iter().map(|s| &**s).collect();
            target.store.create_node_with_id(node.id, &label_refs)?;

            // Log to WAL
            target.log_wal(&WalRecord::CreateNode {
                id: node.id,
                labels: node.labels.iter().map(|s| s.to_string()).collect(),
            })?;

            // Copy properties
            for (key, value) in node.properties {
                target
                    .store
                    .set_node_property(node.id, key.as_str(), value.clone());
                target.log_wal(&WalRecord::SetNodeProperty {
                    id: node.id,
                    key: key.to_string(),
                    value,
                })?;
            }
        }

        // Copy all edges using WAL-enabled methods
        for edge in self.store.all_edges() {
            target
                .store
                .create_edge_with_id(edge.id, edge.src, edge.dst, &edge.edge_type)?;

            // Log to WAL
            target.log_wal(&WalRecord::CreateEdge {
                id: edge.id,
                src: edge.src,
                dst: edge.dst,
                edge_type: edge.edge_type.to_string(),
            })?;

            // Copy properties
            for (key, value) in edge.properties {
                target
                    .store
                    .set_edge_property(edge.id, key.as_str(), value.clone());
                target.log_wal(&WalRecord::SetEdgeProperty {
                    id: edge.id,
                    key: key.to_string(),
                    value,
                })?;
            }
        }

        // Copy named graphs
        for graph_name in self.store.graph_names() {
            if let Some(src_graph) = self.store.graph(&graph_name) {
                target.log_wal(&WalRecord::CreateNamedGraph {
                    name: graph_name.clone(),
                })?;
                target
                    .store
                    .create_graph(&graph_name)
                    .map_err(|e| Error::Internal(e.to_string()))?;

                if let Some(dst_graph) = target.store.graph(&graph_name) {
                    // Switch WAL context to this named graph
                    target.log_wal(&WalRecord::SwitchGraph {
                        name: Some(graph_name.clone()),
                    })?;

                    for node in src_graph.all_nodes() {
                        let label_refs: Vec<&str> = node.labels.iter().map(|s| &**s).collect();
                        dst_graph.create_node_with_id(node.id, &label_refs)?;
                        target.log_wal(&WalRecord::CreateNode {
                            id: node.id,
                            labels: node.labels.iter().map(|s| s.to_string()).collect(),
                        })?;
                        for (key, value) in node.properties {
                            dst_graph.set_node_property(node.id, key.as_str(), value.clone());
                            target.log_wal(&WalRecord::SetNodeProperty {
                                id: node.id,
                                key: key.to_string(),
                                value,
                            })?;
                        }
                    }
                    for edge in src_graph.all_edges() {
                        dst_graph.create_edge_with_id(
                            edge.id,
                            edge.src,
                            edge.dst,
                            &edge.edge_type,
                        )?;
                        target.log_wal(&WalRecord::CreateEdge {
                            id: edge.id,
                            src: edge.src,
                            dst: edge.dst,
                            edge_type: edge.edge_type.to_string(),
                        })?;
                        for (key, value) in edge.properties {
                            dst_graph.set_edge_property(edge.id, key.as_str(), value.clone());
                            target.log_wal(&WalRecord::SetEdgeProperty {
                                id: edge.id,
                                key: key.to_string(),
                                value,
                            })?;
                        }
                    }
                }
            }
        }

        // Switch WAL context back to default graph
        if !self.store.graph_names().is_empty() {
            target.log_wal(&WalRecord::SwitchGraph { name: None })?;
        }

        // Checkpoint and close the target database
        target.close()?;

        Ok(())
    }

    /// Creates an in-memory copy of this database.
    ///
    /// Returns a new database that is completely independent, including
    /// all named graph data.
    /// Useful for:
    /// - Testing modifications without affecting the original
    /// - Faster operations when persistence isn't needed
    ///
    /// # Errors
    ///
    /// Returns an error if the copy operation fails.
    pub fn to_memory(&self) -> Result<Self> {
        let config = Config::in_memory();
        let target = Self::with_config(config)?;

        // Copy default graph nodes
        for node in self.store.all_nodes() {
            let label_refs: Vec<&str> = node.labels.iter().map(|s| &**s).collect();
            target.store.create_node_with_id(node.id, &label_refs)?;
            for (key, value) in node.properties {
                target.store.set_node_property(node.id, key.as_str(), value);
            }
        }

        // Copy default graph edges
        for edge in self.store.all_edges() {
            target
                .store
                .create_edge_with_id(edge.id, edge.src, edge.dst, &edge.edge_type)?;
            for (key, value) in edge.properties {
                target.store.set_edge_property(edge.id, key.as_str(), value);
            }
        }

        // Copy named graphs
        for graph_name in self.store.graph_names() {
            if let Some(src_graph) = self.store.graph(&graph_name) {
                target
                    .store
                    .create_graph(&graph_name)
                    .map_err(|e| Error::Internal(e.to_string()))?;
                if let Some(dst_graph) = target.store.graph(&graph_name) {
                    for node in src_graph.all_nodes() {
                        let label_refs: Vec<&str> = node.labels.iter().map(|s| &**s).collect();
                        dst_graph.create_node_with_id(node.id, &label_refs)?;
                        for (key, value) in node.properties {
                            dst_graph.set_node_property(node.id, key.as_str(), value);
                        }
                    }
                    for edge in src_graph.all_edges() {
                        dst_graph.create_edge_with_id(
                            edge.id,
                            edge.src,
                            edge.dst,
                            &edge.edge_type,
                        )?;
                        for (key, value) in edge.properties {
                            dst_graph.set_edge_property(edge.id, key.as_str(), value);
                        }
                    }
                }
            }
        }

        Ok(target)
    }

    /// Opens a database file and loads it entirely into memory.
    ///
    /// The returned database has no connection to the original file.
    /// Changes will NOT be written back to the file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file can't be opened or loaded.
    #[cfg(feature = "wal")]
    pub fn open_in_memory(path: impl AsRef<Path>) -> Result<Self> {
        // Open the source database (triggers WAL recovery)
        let source = Self::open(path)?;

        // Create in-memory copy
        let target = source.to_memory()?;

        // Close the source (releases file handles)
        source.close()?;

        Ok(target)
    }

    // =========================================================================
    // ADMIN API: Snapshot Export/Import
    // =========================================================================

    /// Exports the entire database to a binary snapshot (v2 format).
    ///
    /// The returned bytes can be stored (e.g. in IndexedDB) and later
    /// restored with [`import_snapshot()`](Self::import_snapshot).
    /// Includes all named graph data.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn export_snapshot(&self) -> Result<Vec<u8>> {
        let nodes = collect_snapshot_nodes(&self.store);
        let edges = collect_snapshot_edges(&self.store);

        // Collect named graphs
        let named_graphs: Vec<NamedGraphSnapshot> = self
            .store
            .graph_names()
            .into_iter()
            .filter_map(|name| {
                self.store
                    .graph(&name)
                    .map(|graph_store| NamedGraphSnapshot {
                        name,
                        nodes: collect_snapshot_nodes(&graph_store),
                        edges: collect_snapshot_edges(&graph_store),
                    })
            })
            .collect();

        let snapshot = SnapshotV2 {
            version: 2,
            nodes,
            edges,
            named_graphs,
        };

        let config = bincode::config::standard();
        bincode::serde::encode_to_vec(&snapshot, config)
            .map_err(|e| Error::Internal(format!("snapshot export failed: {e}")))
    }

    /// Creates a new in-memory database from a binary snapshot.
    ///
    /// Accepts both v1 (no named graphs) and v2 (with named graphs) formats.
    /// The `data` must have been produced by [`export_snapshot()`](Self::export_snapshot).
    ///
    /// All edge references are validated before any data is inserted: every
    /// edge's source and destination must reference a node present in the
    /// snapshot, and duplicate node/edge IDs are rejected. If validation
    /// fails, no database is created.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot is invalid, contains dangling edge
    /// references, has duplicate IDs, or deserialization fails.
    pub fn import_snapshot(data: &[u8]) -> Result<Self> {
        if data.is_empty() {
            return Err(Error::Internal("empty snapshot data".to_string()));
        }

        let config = bincode::config::standard();

        // Peek at version byte (bincode standard encodes u8 as raw byte)
        match data[0] {
            1 => Self::import_snapshot_v1(data, config),
            2 => Self::import_snapshot_v2(data, config),
            v => Err(Error::Internal(format!(
                "unsupported snapshot version: {v}"
            ))),
        }
    }

    fn import_snapshot_v1(data: &[u8], config: bincode::config::Configuration) -> Result<Self> {
        let (snapshot, _): (Snapshot, _) = bincode::serde::decode_from_slice(data, config)
            .map_err(|e| Error::Internal(format!("snapshot import failed: {e}")))?;

        validate_snapshot_data(&snapshot.nodes, &snapshot.edges)?;

        let db = Self::new_in_memory();
        populate_store_from_snapshot(&db.store, snapshot.nodes, snapshot.edges)?;
        Ok(db)
    }

    fn import_snapshot_v2(data: &[u8], config: bincode::config::Configuration) -> Result<Self> {
        let (snapshot, _): (SnapshotV2, _) = bincode::serde::decode_from_slice(data, config)
            .map_err(|e| Error::Internal(format!("snapshot import failed: {e}")))?;

        // Validate default graph data
        validate_snapshot_data(&snapshot.nodes, &snapshot.edges)?;

        // Validate each named graph
        for ng in &snapshot.named_graphs {
            validate_snapshot_data(&ng.nodes, &ng.edges)?;
        }

        let db = Self::new_in_memory();
        populate_store_from_snapshot(&db.store, snapshot.nodes, snapshot.edges)?;

        // Restore named graphs
        for ng in snapshot.named_graphs {
            db.store
                .create_graph(&ng.name)
                .map_err(|e| Error::Internal(e.to_string()))?;
            if let Some(graph_store) = db.store.graph(&ng.name) {
                populate_store_from_snapshot(&graph_store, ng.nodes, ng.edges)?;
            }
        }

        Ok(db)
    }

    /// Replaces the current database contents with data from a binary snapshot.
    ///
    /// Accepts both v1 and v2 snapshot formats. The `data` must have been
    /// produced by [`export_snapshot()`](Self::export_snapshot).
    ///
    /// All validation (duplicate IDs, dangling edge references) is performed
    /// before any data is modified. If validation fails, the current database
    /// is left unchanged. If validation passes, the store is cleared and
    /// rebuilt from the snapshot atomically (from the perspective of
    /// subsequent queries).
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot is invalid, contains dangling edge
    /// references, has duplicate IDs, or deserialization fails.
    pub fn restore_snapshot(&self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Err(Error::Internal("empty snapshot data".to_string()));
        }

        let config = bincode::config::standard();

        match data[0] {
            1 => self.restore_snapshot_v1(data, config),
            2 => self.restore_snapshot_v2(data, config),
            v => Err(Error::Internal(format!(
                "unsupported snapshot version: {v}"
            ))),
        }
    }

    fn restore_snapshot_v1(
        &self,
        data: &[u8],
        config: bincode::config::Configuration,
    ) -> Result<()> {
        let (snapshot, _): (Snapshot, _) = bincode::serde::decode_from_slice(data, config)
            .map_err(|e| Error::Internal(format!("snapshot restore failed: {e}")))?;

        validate_snapshot_data(&snapshot.nodes, &snapshot.edges)?;

        // Drop all named graphs before clearing the default store
        for name in self.store.graph_names() {
            self.store.drop_graph(&name);
        }
        self.store.clear();
        populate_store_from_snapshot(&self.store, snapshot.nodes, snapshot.edges)
    }

    fn restore_snapshot_v2(
        &self,
        data: &[u8],
        config: bincode::config::Configuration,
    ) -> Result<()> {
        let (snapshot, _): (SnapshotV2, _) = bincode::serde::decode_from_slice(data, config)
            .map_err(|e| Error::Internal(format!("snapshot restore failed: {e}")))?;

        // Validate all data before making any changes
        validate_snapshot_data(&snapshot.nodes, &snapshot.edges)?;
        for ng in &snapshot.named_graphs {
            validate_snapshot_data(&ng.nodes, &ng.edges)?;
        }

        // Drop all existing named graphs, then clear default store
        for name in self.store.graph_names() {
            self.store.drop_graph(&name);
        }
        self.store.clear();

        populate_store_from_snapshot(&self.store, snapshot.nodes, snapshot.edges)?;

        // Restore named graphs
        for ng in snapshot.named_graphs {
            self.store
                .create_graph(&ng.name)
                .map_err(|e| Error::Internal(e.to_string()))?;
            if let Some(graph_store) = self.store.graph(&ng.name) {
                populate_store_from_snapshot(&graph_store, ng.nodes, ng.edges)?;
            }
        }

        Ok(())
    }

    // =========================================================================
    // ADMIN API: Iteration
    // =========================================================================

    /// Returns an iterator over all nodes in the database.
    ///
    /// Useful for dump/export operations.
    pub fn iter_nodes(&self) -> impl Iterator<Item = grafeo_core::graph::lpg::Node> + '_ {
        self.store.all_nodes()
    }

    /// Returns an iterator over all edges in the database.
    ///
    /// Useful for dump/export operations.
    pub fn iter_edges(&self) -> impl Iterator<Item = grafeo_core::graph::lpg::Edge> + '_ {
        self.store.all_edges()
    }
}

#[cfg(test)]
mod tests {
    use grafeo_common::types::{EdgeId, NodeId, Value};

    use super::super::GrafeoDB;
    use super::{Snapshot, SnapshotEdge, SnapshotNode};

    #[test]
    fn test_restore_snapshot_basic() {
        let db = GrafeoDB::new_in_memory();
        let session = db.session();

        // Populate
        session.execute("INSERT (:Person {name: 'Alix'})").unwrap();
        session.execute("INSERT (:Person {name: 'Gus'})").unwrap();

        let snapshot = db.export_snapshot().unwrap();

        // Modify
        session
            .execute("INSERT (:Person {name: 'Vincent'})")
            .unwrap();
        assert_eq!(db.store.node_count(), 3);

        // Restore original
        db.restore_snapshot(&snapshot).unwrap();

        assert_eq!(db.store.node_count(), 2);
        let result = session.execute("MATCH (n:Person) RETURN n.name").unwrap();
        assert_eq!(result.rows.len(), 2);
    }

    #[test]
    fn test_restore_snapshot_validation_failure() {
        let db = GrafeoDB::new_in_memory();
        let session = db.session();

        session.execute("INSERT (:Person {name: 'Alix'})").unwrap();

        // Corrupt snapshot: just garbage bytes
        let result = db.restore_snapshot(b"garbage");
        assert!(result.is_err());

        // DB should be unchanged
        assert_eq!(db.store.node_count(), 1);
    }

    #[test]
    fn test_restore_snapshot_empty_db() {
        let db = GrafeoDB::new_in_memory();

        // Export empty snapshot, then populate, then restore to empty
        let empty_snapshot = db.export_snapshot().unwrap();

        let session = db.session();
        session.execute("INSERT (:Person {name: 'Alix'})").unwrap();
        assert_eq!(db.store.node_count(), 1);

        db.restore_snapshot(&empty_snapshot).unwrap();
        assert_eq!(db.store.node_count(), 0);
    }

    #[test]
    fn test_restore_snapshot_with_edges() {
        let db = GrafeoDB::new_in_memory();
        let session = db.session();

        session.execute("INSERT (:Person {name: 'Alix'})").unwrap();
        session.execute("INSERT (:Person {name: 'Gus'})").unwrap();
        session
            .execute(
                "MATCH (a:Person {name: 'Alix'}), (b:Person {name: 'Gus'}) INSERT (a)-[:KNOWS]->(b)",
            )
            .unwrap();

        let snapshot = db.export_snapshot().unwrap();
        assert_eq!(db.store.edge_count(), 1);

        // Modify: add more data
        session
            .execute("INSERT (:Person {name: 'Vincent'})")
            .unwrap();

        // Restore
        db.restore_snapshot(&snapshot).unwrap();
        assert_eq!(db.store.node_count(), 2);
        assert_eq!(db.store.edge_count(), 1);
    }

    #[test]
    fn test_restore_snapshot_preserves_sessions() {
        let db = GrafeoDB::new_in_memory();
        let session = db.session();

        session.execute("INSERT (:Person {name: 'Alix'})").unwrap();
        let snapshot = db.export_snapshot().unwrap();

        // Modify
        session.execute("INSERT (:Person {name: 'Gus'})").unwrap();

        // Restore
        db.restore_snapshot(&snapshot).unwrap();

        // Session should still work and see restored data
        let result = session.execute("MATCH (n:Person) RETURN n.name").unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    #[test]
    fn test_export_import_roundtrip() {
        let db = GrafeoDB::new_in_memory();
        let session = db.session();

        session
            .execute("INSERT (:Person {name: 'Alix', age: 30})")
            .unwrap();

        let snapshot = db.export_snapshot().unwrap();
        let db2 = GrafeoDB::import_snapshot(&snapshot).unwrap();
        let session2 = db2.session();

        let result = session2.execute("MATCH (n:Person) RETURN n.name").unwrap();
        assert_eq!(result.rows.len(), 1);
    }

    // --- to_memory() ---

    #[test]
    fn test_to_memory_empty() {
        let db = GrafeoDB::new_in_memory();
        let copy = db.to_memory().unwrap();
        assert_eq!(copy.store.node_count(), 0);
        assert_eq!(copy.store.edge_count(), 0);
    }

    #[test]
    fn test_to_memory_copies_nodes_and_properties() {
        let db = GrafeoDB::new_in_memory();
        let session = db.session();
        session
            .execute("INSERT (:Person {name: 'Alix', age: 30})")
            .unwrap();
        session
            .execute("INSERT (:Person {name: 'Gus', age: 25})")
            .unwrap();

        let copy = db.to_memory().unwrap();
        assert_eq!(copy.store.node_count(), 2);

        let s2 = copy.session();
        let result = s2
            .execute("MATCH (p:Person) RETURN p.name ORDER BY p.name")
            .unwrap();
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], Value::String("Alix".into()));
        assert_eq!(result.rows[1][0], Value::String("Gus".into()));
    }

    #[test]
    fn test_to_memory_copies_edges_and_properties() {
        let db = GrafeoDB::new_in_memory();
        let a = db.create_node(&["Person"]);
        db.set_node_property(a, "name", "Alix".into());
        let b = db.create_node(&["Person"]);
        db.set_node_property(b, "name", "Gus".into());
        let edge = db.create_edge(a, b, "KNOWS");
        db.set_edge_property(edge, "since", Value::Int64(2020));

        let copy = db.to_memory().unwrap();
        assert_eq!(copy.store.node_count(), 2);
        assert_eq!(copy.store.edge_count(), 1);

        let s2 = copy.session();
        let result = s2.execute("MATCH ()-[e:KNOWS]->() RETURN e.since").unwrap();
        assert_eq!(result.rows[0][0], Value::Int64(2020));
    }

    #[test]
    fn test_to_memory_is_independent() {
        let db = GrafeoDB::new_in_memory();
        let session = db.session();
        session.execute("INSERT (:Person {name: 'Alix'})").unwrap();

        let copy = db.to_memory().unwrap();

        // Mutating original should not affect copy
        session.execute("INSERT (:Person {name: 'Gus'})").unwrap();
        assert_eq!(db.store.node_count(), 2);
        assert_eq!(copy.store.node_count(), 1);
    }

    // --- iter_nodes() / iter_edges() ---

    #[test]
    fn test_iter_nodes_empty() {
        let db = GrafeoDB::new_in_memory();
        assert_eq!(db.iter_nodes().count(), 0);
    }

    #[test]
    fn test_iter_nodes_returns_all() {
        let db = GrafeoDB::new_in_memory();
        let id1 = db.create_node(&["Person"]);
        db.set_node_property(id1, "name", "Alix".into());
        let id2 = db.create_node(&["Animal"]);
        db.set_node_property(id2, "name", "Fido".into());

        let nodes: Vec<_> = db.iter_nodes().collect();
        assert_eq!(nodes.len(), 2);

        let names: Vec<_> = nodes
            .iter()
            .filter_map(|n| n.properties.iter().find(|(k, _)| k.as_str() == "name"))
            .map(|(_, v)| v.clone())
            .collect();
        assert!(names.contains(&Value::String("Alix".into())));
        assert!(names.contains(&Value::String("Fido".into())));
    }

    #[test]
    fn test_iter_edges_empty() {
        let db = GrafeoDB::new_in_memory();
        assert_eq!(db.iter_edges().count(), 0);
    }

    #[test]
    fn test_iter_edges_returns_all() {
        let db = GrafeoDB::new_in_memory();
        let a = db.create_node(&["A"]);
        let b = db.create_node(&["B"]);
        let c = db.create_node(&["C"]);
        db.create_edge(a, b, "R1");
        db.create_edge(b, c, "R2");

        let edges: Vec<_> = db.iter_edges().collect();
        assert_eq!(edges.len(), 2);

        let types: Vec<_> = edges.iter().map(|e| e.edge_type.as_ref()).collect();
        assert!(types.contains(&"R1"));
        assert!(types.contains(&"R2"));
    }

    // --- restore_snapshot() validation ---

    fn encode_snapshot(snap: &Snapshot) -> Vec<u8> {
        bincode::serde::encode_to_vec(snap, bincode::config::standard()).unwrap()
    }

    #[test]
    fn test_restore_rejects_unsupported_version() {
        let db = GrafeoDB::new_in_memory();
        let session = db.session();
        session.execute("INSERT (:Person {name: 'Alix'})").unwrap();

        let snap = Snapshot {
            version: 99,
            nodes: vec![],
            edges: vec![],
        };
        let bytes = encode_snapshot(&snap);

        let result = db.restore_snapshot(&bytes);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unsupported snapshot version"), "got: {err}");

        // DB unchanged
        assert_eq!(db.store.node_count(), 1);
    }

    #[test]
    fn test_restore_rejects_duplicate_node_ids() {
        let db = GrafeoDB::new_in_memory();
        let session = db.session();
        session.execute("INSERT (:Person {name: 'Alix'})").unwrap();

        let snap = Snapshot {
            version: 1,
            nodes: vec![
                SnapshotNode {
                    id: NodeId::new(0),
                    labels: vec!["A".into()],
                    properties: vec![],
                },
                SnapshotNode {
                    id: NodeId::new(0),
                    labels: vec!["B".into()],
                    properties: vec![],
                },
            ],
            edges: vec![],
        };
        let bytes = encode_snapshot(&snap);

        let result = db.restore_snapshot(&bytes);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("duplicate node ID"), "got: {err}");
        assert_eq!(db.store.node_count(), 1);
    }

    #[test]
    fn test_restore_rejects_duplicate_edge_ids() {
        let db = GrafeoDB::new_in_memory();

        let snap = Snapshot {
            version: 1,
            nodes: vec![
                SnapshotNode {
                    id: NodeId::new(0),
                    labels: vec![],
                    properties: vec![],
                },
                SnapshotNode {
                    id: NodeId::new(1),
                    labels: vec![],
                    properties: vec![],
                },
            ],
            edges: vec![
                SnapshotEdge {
                    id: EdgeId::new(0),
                    src: NodeId::new(0),
                    dst: NodeId::new(1),
                    edge_type: "REL".into(),
                    properties: vec![],
                },
                SnapshotEdge {
                    id: EdgeId::new(0),
                    src: NodeId::new(0),
                    dst: NodeId::new(1),
                    edge_type: "REL".into(),
                    properties: vec![],
                },
            ],
        };
        let bytes = encode_snapshot(&snap);

        let result = db.restore_snapshot(&bytes);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("duplicate edge ID"), "got: {err}");
    }

    #[test]
    fn test_restore_rejects_dangling_source() {
        let db = GrafeoDB::new_in_memory();

        let snap = Snapshot {
            version: 1,
            nodes: vec![SnapshotNode {
                id: NodeId::new(0),
                labels: vec![],
                properties: vec![],
            }],
            edges: vec![SnapshotEdge {
                id: EdgeId::new(0),
                src: NodeId::new(999),
                dst: NodeId::new(0),
                edge_type: "REL".into(),
                properties: vec![],
            }],
        };
        let bytes = encode_snapshot(&snap);

        let result = db.restore_snapshot(&bytes);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("non-existent source node"), "got: {err}");
    }

    #[test]
    fn test_restore_rejects_dangling_destination() {
        let db = GrafeoDB::new_in_memory();

        let snap = Snapshot {
            version: 1,
            nodes: vec![SnapshotNode {
                id: NodeId::new(0),
                labels: vec![],
                properties: vec![],
            }],
            edges: vec![SnapshotEdge {
                id: EdgeId::new(0),
                src: NodeId::new(0),
                dst: NodeId::new(999),
                edge_type: "REL".into(),
                properties: vec![],
            }],
        };
        let bytes = encode_snapshot(&snap);

        let result = db.restore_snapshot(&bytes);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("non-existent destination node"), "got: {err}");
    }
}
