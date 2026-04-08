//! Catalog section serializer for the `.grafeo` container format.
//!
//! Serializes schema definitions (node types, edge types, graph types, procedures),
//! index metadata (property, vector, text), and epoch state into the `CATALOG` section.

// Allowed until Phase 5 wires this into the checkpoint path.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use grafeo_common::storage::section::{Section, SectionType};
use grafeo_common::utils::error::{Error, Result};

use crate::catalog::{
    Catalog, EdgeTypeDefinition, GraphTypeDefinition, NodeTypeDefinition, ProcedureDefinition,
};

/// Current catalog section format version.
const CATALOG_SECTION_VERSION: u8 = 1;

// ── Snapshot types ──────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct CatalogSnapshot {
    version: u8,
    schema: SnapshotSchema,
    indexes: SnapshotIndexes,
    epoch: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct SnapshotSchema {
    node_types: Vec<NodeTypeDefinition>,
    edge_types: Vec<EdgeTypeDefinition>,
    graph_types: Vec<GraphTypeDefinition>,
    procedures: Vec<ProcedureDefinition>,
    schemas: Vec<String>,
    graph_type_bindings: Vec<(String, String)>,
}

#[derive(Serialize, Deserialize, Default)]
struct SnapshotIndexes {
    property_indexes: Vec<String>,
    vector_indexes: Vec<SnapshotVectorIndex>,
    text_indexes: Vec<SnapshotTextIndex>,
}

#[derive(Serialize, Deserialize)]
struct SnapshotVectorIndex {
    label: String,
    property: String,
    dimensions: usize,
    metric: grafeo_core::index::vector::DistanceMetric,
    m: usize,
    ef_construction: usize,
}

#[derive(Serialize, Deserialize)]
struct SnapshotTextIndex {
    label: String,
    property: String,
}

// ── Section implementation ──────────────────────────────────────────

/// Catalog section for the `.grafeo` container.
///
/// Serializes schema definitions and index metadata. The catalog is always
/// small (typically < 10 KB) and always kept in RAM.
pub struct CatalogSection {
    catalog: Arc<Catalog>,
    store: Arc<grafeo_core::graph::lpg::LpgStore>,
    epoch_fn: Box<dyn Fn() -> u64 + Send + Sync>,
    dirty: AtomicBool,
}

impl CatalogSection {
    /// Create a new catalog section.
    ///
    /// The `epoch_fn` closure returns the current MVCC epoch. This avoids a
    /// dependency on `TransactionManager` which lives in the engine layer.
    pub fn new(
        catalog: Arc<Catalog>,
        store: Arc<grafeo_core::graph::lpg::LpgStore>,
        epoch_fn: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            catalog,
            store,
            epoch_fn: Box::new(epoch_fn),
            dirty: AtomicBool::new(false),
        }
    }

    /// Mark this section as dirty.
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    fn collect_schema(&self) -> SnapshotSchema {
        SnapshotSchema {
            node_types: self.catalog.all_node_type_defs(),
            edge_types: self.catalog.all_edge_type_defs(),
            graph_types: self.catalog.all_graph_type_defs(),
            procedures: self.catalog.all_procedure_defs(),
            schemas: self.catalog.schema_names(),
            graph_type_bindings: self.catalog.all_graph_type_bindings(),
        }
    }

    fn collect_indexes(&self) -> SnapshotIndexes {
        let property_indexes = self.store.property_index_keys();

        #[cfg(feature = "vector-index")]
        let vector_indexes: Vec<SnapshotVectorIndex> = self
            .store
            .vector_index_entries()
            .into_iter()
            .filter_map(|(key, index)| {
                let (label, property) = key.split_once(':')?;
                let config = index.config();
                Some(SnapshotVectorIndex {
                    label: label.to_string(),
                    property: property.to_string(),
                    dimensions: config.dimensions,
                    metric: config.metric,
                    m: config.m,
                    ef_construction: config.ef_construction,
                })
            })
            .collect();
        #[cfg(not(feature = "vector-index"))]
        let vector_indexes = Vec::new();

        #[cfg(feature = "text-index")]
        let text_indexes: Vec<SnapshotTextIndex> = self
            .store
            .text_index_entries()
            .into_iter()
            .filter_map(|(key, _)| {
                let (label, property) = key.split_once(':')?;
                Some(SnapshotTextIndex {
                    label: label.to_string(),
                    property: property.to_string(),
                })
            })
            .collect();
        #[cfg(not(feature = "text-index"))]
        let text_indexes = Vec::new();

        SnapshotIndexes {
            property_indexes,
            vector_indexes,
            text_indexes,
        }
    }
}

impl Section for CatalogSection {
    fn section_type(&self) -> SectionType {
        SectionType::Catalog
    }

    fn version(&self) -> u8 {
        CATALOG_SECTION_VERSION
    }

    fn serialize(&self) -> Result<Vec<u8>> {
        let snapshot = CatalogSnapshot {
            version: CATALOG_SECTION_VERSION,
            schema: self.collect_schema(),
            indexes: self.collect_indexes(),
            epoch: (self.epoch_fn)(),
        };

        let config = bincode::config::standard();
        bincode::serde::encode_to_vec(&snapshot, config)
            .map_err(|e| Error::Internal(format!("Catalog section serialization failed: {e}")))
    }

    fn deserialize(&mut self, data: &[u8]) -> Result<()> {
        let config = bincode::config::standard();
        let (snapshot, _): (CatalogSnapshot, _) = bincode::serde::decode_from_slice(data, config)
            .map_err(|e| {
            Error::Serialization(format!("Catalog section deserialization failed: {e}"))
        })?;

        // Restore schema definitions
        for def in &snapshot.schema.node_types {
            self.catalog.register_or_replace_node_type(def.clone());
        }
        for def in &snapshot.schema.edge_types {
            self.catalog.register_or_replace_edge_type_def(def.clone());
        }
        for def in &snapshot.schema.graph_types {
            let _ = self.catalog.register_graph_type(def.clone());
        }
        for def in &snapshot.schema.procedures {
            self.catalog.replace_procedure(def.clone()).ok();
        }
        for name in &snapshot.schema.schemas {
            let _ = self.catalog.register_schema_namespace(name.clone());
            let default_key = format!("{name}/__default__");
            let _ = self.store.create_graph(&default_key);
        }
        for (graph_name, type_name) in &snapshot.schema.graph_type_bindings {
            let _ = self.catalog.bind_graph_type(graph_name, type_name.clone());
        }

        // Index metadata is stored for reference. Actual index rebuilding
        // happens in the engine after all data sections are loaded.
        // The engine reads the catalog's index defs and calls create_*_index.

        Ok(())
    }

    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    fn mark_clean(&self) {
        self.dirty.store(false, Ordering::Release);
    }

    fn memory_usage(&self) -> usize {
        // Catalog is tiny: schema defs + index metadata, typically < 10 KB
        4096
    }
}
