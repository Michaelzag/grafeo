//! Change Data Capture (CDC) for tracking entity mutations.
//!
//! When the `cdc` feature is enabled, the database records every mutation
//! (create, update, delete) with before/after property snapshots. This
//! enables audit trails, temporal queries, and downstream sync.
//!
//! # Example
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use grafeo_engine::GrafeoDB;
//! use grafeo_common::types::Value;
//!
//! let db = GrafeoDB::new_in_memory();
//! let id = db.create_node(&["Person"]);
//! db.set_node_property(id, "name", Value::from("Alix"));
//! db.set_node_property(id, "name", Value::from("Gus"));
//!
//! let history = db.history(id)?;
//! assert_eq!(history.len(), 3); // create + 2 updates
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use grafeo_common::types::{EdgeId, EpochId, HlcClock, HlcTimestamp, NodeId, Value};
use hashbrown::HashMap as HbHashMap;
use parking_lot::RwLock;

/// The kind of mutation that occurred.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ChangeKind {
    /// A new entity was created.
    Create,
    /// An existing entity was updated (property set or removed).
    Update,
    /// An entity was deleted.
    Delete,
}

/// A unique identifier for a graph entity (node, edge, or RDF triple).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EntityId {
    /// A node identifier.
    Node(NodeId),
    /// An edge identifier.
    Edge(EdgeId),
    /// An RDF triple, identified by a content hash of its terms.
    Triple(u64),
}

impl From<NodeId> for EntityId {
    fn from(id: NodeId) -> Self {
        Self::Node(id)
    }
}

impl From<EdgeId> for EntityId {
    fn from(id: EdgeId) -> Self {
        Self::Edge(id)
    }
}

impl EntityId {
    /// Returns the raw u64 value for binding layers.
    #[must_use]
    pub fn as_u64(&self) -> u64 {
        match self {
            Self::Node(id) => id.as_u64(),
            Self::Edge(id) => id.as_u64(),
            Self::Triple(h) => *h,
        }
    }

    /// Returns `true` if this is a node identifier.
    #[must_use]
    pub fn is_node(&self) -> bool {
        matches!(self, Self::Node(_))
    }

    /// Returns `true` if this is an RDF triple identifier.
    #[must_use]
    pub fn is_triple(&self) -> bool {
        matches!(self, Self::Triple(_))
    }
}

/// A recorded change event with before/after property snapshots, or an RDF
/// triple insert/delete.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChangeEvent {
    /// The entity that was changed.
    pub entity_id: EntityId,
    /// The kind of change.
    pub kind: ChangeKind,
    /// MVCC epoch when the change occurred.
    pub epoch: EpochId,
    /// Hybrid Logical Clock timestamp for causal ordering.
    ///
    /// Encodes physical milliseconds (upper 48 bits) and a logical counter
    /// (lower 16 bits) into a `u64`. Backward-compatible: plain wall-clock
    /// values have logical counter = 0.
    pub timestamp: HlcTimestamp,
    /// Properties before the change (None for Create and for triple events).
    pub before: Option<HashMap<String, Value>>,
    /// Properties after the change (None for Delete and for triple events).
    pub after: Option<HashMap<String, Value>>,
    /// Node labels. Present only on node Create events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    /// Edge relationship type. Present only on edge Create events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_type: Option<String>,
    /// Edge source node ID. Present only on edge Create events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src_id: Option<u64>,
    /// Edge destination node ID. Present only on edge Create events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dst_id: Option<u64>,
    /// RDF triple subject (N-Triples encoded). Present only on triple events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triple_subject: Option<String>,
    /// RDF triple predicate (N-Triples encoded). Present only on triple events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triple_predicate: Option<String>,
    /// RDF triple object (N-Triples encoded). Present only on triple events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triple_object: Option<String>,
    /// Named graph containing the triple. `None` means the default graph.
    /// Present only on triple events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triple_graph: Option<String>,
}

/// The CDC log that records entity mutations.
///
/// Thread-safe: uses `RwLock<HashMap>` for concurrent access. Timestamps
/// are assigned by the embedded [`HlcClock`] to guarantee monotonicity.
#[derive(Debug)]
pub struct CdcLog {
    events: RwLock<HbHashMap<EntityId, Vec<ChangeEvent>>>,
    clock: Arc<HlcClock>,
}

impl CdcLog {
    /// Creates a new empty CDC log with a fresh HLC clock.
    #[must_use]
    pub fn new() -> Self {
        Self {
            events: RwLock::new(HbHashMap::new()),
            clock: Arc::new(HlcClock::new()),
        }
    }

    /// Returns the next HLC timestamp from this log's clock.
    ///
    /// Used by `CdcGraphStore` to assign timestamps to buffered events.
    pub fn next_timestamp(&self) -> HlcTimestamp {
        self.clock.now()
    }

    /// Returns a reference to the HLC clock for remote timestamp merging.
    pub fn clock(&self) -> &Arc<HlcClock> {
        &self.clock
    }

    /// Records a change event.
    pub fn record(&self, event: ChangeEvent) {
        self.events
            .write()
            .entry(event.entity_id)
            .or_default()
            .push(event);
    }

    /// Records a batch of change events with a single write-lock acquisition.
    pub fn record_batch(&self, events: impl IntoIterator<Item = ChangeEvent>) {
        let mut guard = self.events.write();
        for event in events {
            guard.entry(event.entity_id).or_default().push(event);
        }
    }

    /// Records a node creation.
    pub fn record_create_node(
        &self,
        id: NodeId,
        epoch: EpochId,
        props: Option<HashMap<String, Value>>,
        labels: Option<Vec<String>>,
    ) {
        self.record(ChangeEvent {
            entity_id: EntityId::Node(id),
            kind: ChangeKind::Create,
            epoch,
            timestamp: self.clock.now(),
            before: None,
            after: props,
            labels,
            edge_type: None,
            src_id: None,
            dst_id: None,
            triple_subject: None,
            triple_predicate: None,
            triple_object: None,
            triple_graph: None,
        });
    }

    /// Records an edge creation.
    pub fn record_create_edge(
        &self,
        id: EdgeId,
        epoch: EpochId,
        props: Option<HashMap<String, Value>>,
        src_id: u64,
        dst_id: u64,
        edge_type: String,
    ) {
        self.record(ChangeEvent {
            entity_id: EntityId::Edge(id),
            kind: ChangeKind::Create,
            epoch,
            timestamp: self.clock.now(),
            before: None,
            after: props,
            labels: None,
            edge_type: Some(edge_type),
            src_id: Some(src_id),
            dst_id: Some(dst_id),
            triple_subject: None,
            triple_predicate: None,
            triple_object: None,
            triple_graph: None,
        });
    }

    /// Records an RDF triple insertion.
    ///
    /// The terms must be N-Triples encoded (e.g. `<http://example.org/s>`,
    /// `"hello"`, `"42"^^<http://www.w3.org/2001/XMLSchema#integer>`).
    pub fn record_triple_insert(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        graph: Option<&str>,
        epoch: EpochId,
    ) {
        let id = triple_hash(subject, predicate, object, graph);
        self.record(ChangeEvent {
            entity_id: EntityId::Triple(id),
            kind: ChangeKind::Create,
            epoch,
            timestamp: self.clock.now(),
            before: None,
            after: None,
            labels: None,
            edge_type: None,
            src_id: None,
            dst_id: None,
            triple_subject: Some(subject.to_string()),
            triple_predicate: Some(predicate.to_string()),
            triple_object: Some(object.to_string()),
            triple_graph: graph.map(ToString::to_string),
        });
    }

    /// Records an RDF triple deletion.
    ///
    /// The terms must be N-Triples encoded.
    pub fn record_triple_delete(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        graph: Option<&str>,
        epoch: EpochId,
    ) {
        let id = triple_hash(subject, predicate, object, graph);
        self.record(ChangeEvent {
            entity_id: EntityId::Triple(id),
            kind: ChangeKind::Delete,
            epoch,
            timestamp: self.clock.now(),
            before: None,
            after: None,
            labels: None,
            edge_type: None,
            src_id: None,
            dst_id: None,
            triple_subject: Some(subject.to_string()),
            triple_predicate: Some(predicate.to_string()),
            triple_object: Some(object.to_string()),
            triple_graph: graph.map(ToString::to_string),
        });
    }

    /// Records a property update.
    pub fn record_update(
        &self,
        entity_id: EntityId,
        epoch: EpochId,
        key: &str,
        old_value: Option<Value>,
        new_value: Value,
    ) {
        let before = old_value.map(|v| {
            let mut m = HashMap::new();
            m.insert(key.to_string(), v);
            m
        });
        let mut after_map = HashMap::new();
        after_map.insert(key.to_string(), new_value);

        self.record(ChangeEvent {
            entity_id,
            kind: ChangeKind::Update,
            epoch,
            timestamp: self.clock.now(),
            before,
            after: Some(after_map),
            labels: None,
            edge_type: None,
            src_id: None,
            dst_id: None,
            triple_subject: None,
            triple_predicate: None,
            triple_object: None,
            triple_graph: None,
        });
    }

    /// Records an entity deletion.
    pub fn record_delete(
        &self,
        entity_id: EntityId,
        epoch: EpochId,
        props: Option<HashMap<String, Value>>,
    ) {
        self.record(ChangeEvent {
            entity_id,
            kind: ChangeKind::Delete,
            epoch,
            timestamp: self.clock.now(),
            before: props,
            after: None,
            labels: None,
            edge_type: None,
            src_id: None,
            dst_id: None,
            triple_subject: None,
            triple_predicate: None,
            triple_object: None,
            triple_graph: None,
        });
    }

    /// Returns all change events for an entity, ordered by epoch.
    #[must_use]
    pub fn history(&self, entity_id: EntityId) -> Vec<ChangeEvent> {
        self.events
            .read()
            .get(&entity_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns change events for an entity since the given epoch.
    #[must_use]
    pub fn history_since(&self, entity_id: EntityId, since_epoch: EpochId) -> Vec<ChangeEvent> {
        self.events
            .read()
            .get(&entity_id)
            .map(|events| {
                events
                    .iter()
                    .filter(|e| e.epoch >= since_epoch)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns all change events across all entities in an epoch range.
    #[must_use]
    pub fn changes_between(&self, start_epoch: EpochId, end_epoch: EpochId) -> Vec<ChangeEvent> {
        let guard = self.events.read();
        let mut results = Vec::new();
        for events in guard.values() {
            for event in events {
                if event.epoch >= start_epoch && event.epoch <= end_epoch {
                    results.push(event.clone());
                }
            }
        }
        results.sort_by_key(|e| e.epoch);
        results
    }

    /// Returns the total number of recorded events.
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.events.read().values().map(Vec::len).sum()
    }
}

impl Default for CdcLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes a stable-within-process content hash for an RDF triple.
///
/// Used as the raw `u64` in `EntityId::Triple` so the CDC log can key events
/// by triple content without storing a separate ID registry.
fn triple_hash(subject: &str, predicate: &str, object: &str, graph: Option<&str>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    subject.hash(&mut h);
    predicate.hash(&mut h);
    object.hash(&mut h);
    graph.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_history() {
        let log = CdcLog::new();
        let node_id = NodeId::new(1);

        log.record_create_node(node_id, EpochId(1), None, None);
        log.record_update(
            EntityId::Node(node_id),
            EpochId(2),
            "name",
            None,
            Value::from("Alix"),
        );
        log.record_update(
            EntityId::Node(node_id),
            EpochId(3),
            "name",
            Some(Value::from("Alix")),
            Value::from("Gus"),
        );

        let history = log.history(EntityId::Node(node_id));
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].kind, ChangeKind::Create);
        assert_eq!(history[1].kind, ChangeKind::Update);
        assert_eq!(history[2].kind, ChangeKind::Update);
    }

    #[test]
    fn test_history_since() {
        let log = CdcLog::new();
        let node_id = NodeId::new(1);

        log.record_create_node(node_id, EpochId(1), None, None);
        log.record_update(
            EntityId::Node(node_id),
            EpochId(5),
            "name",
            None,
            Value::from("Alix"),
        );
        log.record_update(
            EntityId::Node(node_id),
            EpochId(10),
            "name",
            Some(Value::from("Alix")),
            Value::from("Gus"),
        );

        let since_5 = log.history_since(EntityId::Node(node_id), EpochId(5));
        assert_eq!(since_5.len(), 2);
        assert_eq!(since_5[0].epoch, EpochId(5));
    }

    #[test]
    fn test_changes_between() {
        let log = CdcLog::new();

        log.record_create_node(NodeId::new(1), EpochId(1), None, None);
        log.record_create_node(NodeId::new(2), EpochId(3), None, None);
        log.record_update(
            EntityId::Node(NodeId::new(1)),
            EpochId(5),
            "x",
            None,
            Value::from(42),
        );

        let changes = log.changes_between(EpochId(2), EpochId(5));
        assert_eq!(changes.len(), 2); // epoch 3 and 5
    }

    #[test]
    fn test_delete_event() {
        let log = CdcLog::new();
        let node_id = NodeId::new(1);

        let mut props = HashMap::new();
        props.insert("name".to_string(), Value::from("Alix"));

        log.record_create_node(node_id, EpochId(1), Some(props.clone()), None);
        log.record_delete(EntityId::Node(node_id), EpochId(2), Some(props));

        let history = log.history(EntityId::Node(node_id));
        assert_eq!(history.len(), 2);
        assert_eq!(history[1].kind, ChangeKind::Delete);
        assert!(history[1].after.is_none());
        assert!(history[1].before.is_some());
    }

    #[test]
    fn test_empty_history() {
        let log = CdcLog::new();
        let history = log.history(EntityId::Node(NodeId::new(999)));
        assert!(history.is_empty());
    }

    #[test]
    fn test_event_count() {
        let log = CdcLog::new();
        assert_eq!(log.event_count(), 0);

        log.record_create_node(NodeId::new(1), EpochId(1), None, None);
        log.record_create_node(NodeId::new(2), EpochId(2), None, None);
        assert_eq!(log.event_count(), 2);
    }

    #[test]
    fn test_entity_id_conversions() {
        let node_id = NodeId::new(42);
        let entity: EntityId = node_id.into();
        assert!(entity.is_node());
        assert_eq!(entity.as_u64(), 42);

        let edge_id = EdgeId::new(7);
        let entity: EntityId = edge_id.into();
        assert!(!entity.is_node());
        assert_eq!(entity.as_u64(), 7);
    }
}
