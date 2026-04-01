//! CDC-aware graph store wrapper.
//!
//! Wraps a [`GraphStoreMut`] and buffers CDC events for every mutation.
//! Events are held in a transactional buffer (`pending_events`) that the
//! session flushes to the [`CdcLog`] on commit or discards on rollback.
//!
//! This mirrors the [`WalGraphStore`](super::wal_store::WalGraphStore)
//! decorator pattern but targets the CDC audit trail instead of WAL
//! durability.

use std::collections::HashMap;
use std::sync::Arc;

use arcstr::ArcStr;
use grafeo_common::types::{
    EdgeId, EpochId, HlcTimestamp, NodeId, PropertyKey, TransactionId, Value,
};
use grafeo_common::utils::hash::FxHashMap;
use grafeo_core::graph::lpg::{CompareOp, Edge, Node};
use grafeo_core::graph::{Direction, GraphStore, GraphStoreMut};
use grafeo_core::statistics::Statistics;
use parking_lot::Mutex;

use crate::cdc::{CdcLog, ChangeEvent, ChangeKind, EntityId};

/// A [`GraphStoreMut`] decorator that buffers CDC events for every mutation.
///
/// Read-only methods are forwarded to the inner store without CDC interaction.
///
/// Versioned (transactional) mutations buffer events into `pending_events`.
/// The owning session flushes this buffer to `CdcLog` on commit or clears it
/// on rollback.
///
/// Non-versioned mutations (used by the direct CRUD API) record directly to
/// `CdcLog` since they have no transaction context and are immediately visible.
pub(crate) struct CdcGraphStore {
    inner: Arc<dyn GraphStoreMut>,
    cdc_log: Arc<CdcLog>,
    /// Buffered events for the current transaction.
    pending_events: Arc<Mutex<Vec<ChangeEvent>>>,
}

impl CdcGraphStore {
    /// Creates a new CDC-aware store with a fresh event buffer.
    pub fn new(inner: Arc<dyn GraphStoreMut>, cdc_log: Arc<CdcLog>) -> Self {
        Self {
            inner,
            cdc_log,
            pending_events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Wraps a store sharing an existing event buffer.
    ///
    /// Used for named graphs so all mutations in a transaction (across
    /// default and named graphs) buffer to the same `Vec` for atomic
    /// flush/discard.
    pub fn wrap(
        inner: Arc<dyn GraphStoreMut>,
        cdc_log: Arc<CdcLog>,
        pending_events: Arc<Mutex<Vec<ChangeEvent>>>,
    ) -> Self {
        Self {
            inner,
            cdc_log,
            pending_events,
        }
    }

    /// Returns a handle to the pending events buffer.
    pub fn pending_events(&self) -> Arc<Mutex<Vec<ChangeEvent>>> {
        Arc::clone(&self.pending_events)
    }

    /// Buffers a CDC event for later flush on commit.
    ///
    /// The epoch is always set to `PENDING`: the real commit epoch is assigned
    /// when the session flushes the buffer in `commit_inner()`. This ensures
    /// each transaction's events get the unique epoch from `fetch_add(1, SeqCst)`.
    fn buffer_event(&self, mut event: ChangeEvent) {
        event.epoch = EpochId::PENDING;
        self.pending_events.lock().push(event);
    }

    /// Records a CDC event directly (for non-versioned/auto-commit mutations).
    fn record_directly(&self, event: ChangeEvent) {
        self.cdc_log.record(event);
    }

    /// Collects all properties of a node as a `HashMap` for before/after snapshots.
    fn collect_node_properties(&self, id: NodeId) -> Option<HashMap<String, Value>> {
        let node = self.inner.get_node(id)?;
        let map: HashMap<String, Value> = node
            .properties
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.clone()))
            .collect();
        if map.is_empty() { None } else { Some(map) }
    }

    /// Collects all properties of an edge as a `HashMap` for before/after snapshots.
    fn collect_edge_properties(&self, id: EdgeId) -> Option<HashMap<String, Value>> {
        let edge = self.inner.get_edge(id)?;
        let map: HashMap<String, Value> = edge
            .properties
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.clone()))
            .collect();
        if map.is_empty() { None } else { Some(map) }
    }

    /// Collects labels for a node.
    fn collect_node_labels(&self, id: NodeId) -> Option<Vec<String>> {
        let node = self.inner.get_node(id)?;
        Some(node.labels.iter().map(|l| l.to_string()).collect())
    }

    /// Returns the next HLC timestamp from the CDC log's clock.
    fn next_ts(&self) -> HlcTimestamp {
        self.cdc_log.next_timestamp()
    }
}

fn make_event(
    entity_id: EntityId,
    kind: ChangeKind,
    epoch: EpochId,
    timestamp: HlcTimestamp,
) -> ChangeEvent {
    ChangeEvent {
        entity_id,
        kind,
        epoch,
        timestamp,
        before: None,
        after: None,
        labels: None,
        edge_type: None,
        src_id: None,
        dst_id: None,
        triple_subject: None,
        triple_predicate: None,
        triple_object: None,
        triple_graph: None,
    }
}

// ---------------------------------------------------------------------------
// GraphStore (read-only): pure delegation
// ---------------------------------------------------------------------------

impl GraphStore for CdcGraphStore {
    fn get_node(&self, id: NodeId) -> Option<Node> {
        self.inner.get_node(id)
    }

    fn get_edge(&self, id: EdgeId) -> Option<Edge> {
        self.inner.get_edge(id)
    }

    fn get_node_versioned(
        &self,
        id: NodeId,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> Option<Node> {
        self.inner.get_node_versioned(id, epoch, transaction_id)
    }

    fn get_edge_versioned(
        &self,
        id: EdgeId,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> Option<Edge> {
        self.inner.get_edge_versioned(id, epoch, transaction_id)
    }

    fn get_node_at_epoch(&self, id: NodeId, epoch: EpochId) -> Option<Node> {
        self.inner.get_node_at_epoch(id, epoch)
    }

    fn get_edge_at_epoch(&self, id: EdgeId, epoch: EpochId) -> Option<Edge> {
        self.inner.get_edge_at_epoch(id, epoch)
    }

    fn get_node_property(&self, id: NodeId, key: &PropertyKey) -> Option<Value> {
        self.inner.get_node_property(id, key)
    }

    fn get_edge_property(&self, id: EdgeId, key: &PropertyKey) -> Option<Value> {
        self.inner.get_edge_property(id, key)
    }

    fn get_node_property_batch(&self, ids: &[NodeId], key: &PropertyKey) -> Vec<Option<Value>> {
        self.inner.get_node_property_batch(ids, key)
    }

    fn get_nodes_properties_batch(&self, ids: &[NodeId]) -> Vec<FxHashMap<PropertyKey, Value>> {
        self.inner.get_nodes_properties_batch(ids)
    }

    fn get_nodes_properties_selective_batch(
        &self,
        ids: &[NodeId],
        keys: &[PropertyKey],
    ) -> Vec<FxHashMap<PropertyKey, Value>> {
        self.inner.get_nodes_properties_selective_batch(ids, keys)
    }

    fn get_edges_properties_selective_batch(
        &self,
        ids: &[EdgeId],
        keys: &[PropertyKey],
    ) -> Vec<FxHashMap<PropertyKey, Value>> {
        self.inner.get_edges_properties_selective_batch(ids, keys)
    }

    fn neighbors(&self, node: NodeId, direction: Direction) -> Vec<NodeId> {
        self.inner.neighbors(node, direction)
    }

    fn edges_from(&self, node: NodeId, direction: Direction) -> Vec<(NodeId, EdgeId)> {
        self.inner.edges_from(node, direction)
    }

    fn out_degree(&self, node: NodeId) -> usize {
        self.inner.out_degree(node)
    }

    fn in_degree(&self, node: NodeId) -> usize {
        self.inner.in_degree(node)
    }

    fn has_backward_adjacency(&self) -> bool {
        self.inner.has_backward_adjacency()
    }

    fn node_ids(&self) -> Vec<NodeId> {
        self.inner.node_ids()
    }

    fn all_node_ids(&self) -> Vec<NodeId> {
        self.inner.all_node_ids()
    }

    fn nodes_by_label(&self, label: &str) -> Vec<NodeId> {
        self.inner.nodes_by_label(label)
    }

    fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }

    fn edge_type(&self, id: EdgeId) -> Option<ArcStr> {
        self.inner.edge_type(id)
    }

    fn has_property_index(&self, property: &str) -> bool {
        self.inner.has_property_index(property)
    }

    fn find_nodes_by_property(&self, property: &str, value: &Value) -> Vec<NodeId> {
        self.inner.find_nodes_by_property(property, value)
    }

    fn find_nodes_by_properties(&self, conditions: &[(&str, Value)]) -> Vec<NodeId> {
        self.inner.find_nodes_by_properties(conditions)
    }

    fn find_nodes_in_range(
        &self,
        property: &str,
        min: Option<&Value>,
        max: Option<&Value>,
        min_inclusive: bool,
        max_inclusive: bool,
    ) -> Vec<NodeId> {
        self.inner
            .find_nodes_in_range(property, min, max, min_inclusive, max_inclusive)
    }

    fn node_property_might_match(
        &self,
        property: &PropertyKey,
        op: CompareOp,
        value: &Value,
    ) -> bool {
        self.inner.node_property_might_match(property, op, value)
    }

    fn edge_property_might_match(
        &self,
        property: &PropertyKey,
        op: CompareOp,
        value: &Value,
    ) -> bool {
        self.inner.edge_property_might_match(property, op, value)
    }

    fn statistics(&self) -> Arc<Statistics> {
        self.inner.statistics()
    }

    fn estimate_label_cardinality(&self, label: &str) -> f64 {
        self.inner.estimate_label_cardinality(label)
    }

    fn estimate_avg_degree(&self, edge_type: &str, outgoing: bool) -> f64 {
        self.inner.estimate_avg_degree(edge_type, outgoing)
    }

    fn current_epoch(&self) -> EpochId {
        self.inner.current_epoch()
    }

    fn all_labels(&self) -> Vec<String> {
        self.inner.all_labels()
    }

    fn all_edge_types(&self) -> Vec<String> {
        self.inner.all_edge_types()
    }

    fn all_property_keys(&self) -> Vec<String> {
        self.inner.all_property_keys()
    }

    fn is_node_visible_at_epoch(&self, id: NodeId, epoch: EpochId) -> bool {
        self.inner.is_node_visible_at_epoch(id, epoch)
    }

    fn is_node_visible_versioned(
        &self,
        id: NodeId,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> bool {
        self.inner
            .is_node_visible_versioned(id, epoch, transaction_id)
    }

    fn is_edge_visible_at_epoch(&self, id: EdgeId, epoch: EpochId) -> bool {
        self.inner.is_edge_visible_at_epoch(id, epoch)
    }

    fn is_edge_visible_versioned(
        &self,
        id: EdgeId,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> bool {
        self.inner
            .is_edge_visible_versioned(id, epoch, transaction_id)
    }

    fn filter_visible_node_ids(&self, ids: &[NodeId], epoch: EpochId) -> Vec<NodeId> {
        self.inner.filter_visible_node_ids(ids, epoch)
    }

    fn filter_visible_node_ids_versioned(
        &self,
        ids: &[NodeId],
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> Vec<NodeId> {
        self.inner
            .filter_visible_node_ids_versioned(ids, epoch, transaction_id)
    }

    fn get_node_history(&self, id: NodeId) -> Vec<(EpochId, Option<EpochId>, Node)> {
        self.inner.get_node_history(id)
    }

    fn get_edge_history(&self, id: EdgeId) -> Vec<(EpochId, Option<EpochId>, Edge)> {
        self.inner.get_edge_history(id)
    }
}

// ---------------------------------------------------------------------------
// GraphStoreMut: delegate + CDC buffer/record
// ---------------------------------------------------------------------------

impl GraphStoreMut for CdcGraphStore {
    // --- Node creation ---

    fn create_node(&self, labels: &[&str]) -> NodeId {
        let id = self.inner.create_node(labels);
        let epoch = self.inner.current_epoch();
        let mut event = make_event(
            EntityId::Node(id),
            ChangeKind::Create,
            epoch,
            self.next_ts(),
        );
        event.labels = Some(labels.iter().map(|s| (*s).to_string()).collect());
        self.record_directly(event);
        id
    }

    fn create_node_versioned(
        &self,
        labels: &[&str],
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> NodeId {
        let id = self
            .inner
            .create_node_versioned(labels, epoch, transaction_id);
        // Use PENDING epoch: the real commit epoch is assigned during flush.
        let mut event = make_event(
            EntityId::Node(id),
            ChangeKind::Create,
            EpochId::PENDING,
            self.next_ts(),
        );
        event.labels = Some(labels.iter().map(|s| (*s).to_string()).collect());
        self.buffer_event(event);
        id
    }

    // --- Edge creation ---

    fn create_edge(&self, src: NodeId, dst: NodeId, edge_type: &str) -> EdgeId {
        let id = self.inner.create_edge(src, dst, edge_type);
        let epoch = self.inner.current_epoch();
        let mut event = make_event(
            EntityId::Edge(id),
            ChangeKind::Create,
            epoch,
            self.next_ts(),
        );
        event.edge_type = Some(edge_type.to_string());
        event.src_id = Some(src.as_u64());
        event.dst_id = Some(dst.as_u64());
        self.record_directly(event);
        id
    }

    fn create_edge_versioned(
        &self,
        src: NodeId,
        dst: NodeId,
        edge_type: &str,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> EdgeId {
        let id = self
            .inner
            .create_edge_versioned(src, dst, edge_type, epoch, transaction_id);
        let mut event = make_event(
            EntityId::Edge(id),
            ChangeKind::Create,
            epoch,
            self.next_ts(),
        );
        event.edge_type = Some(edge_type.to_string());
        event.src_id = Some(src.as_u64());
        event.dst_id = Some(dst.as_u64());
        self.buffer_event(event);
        id
    }

    fn batch_create_edges(&self, edges: &[(NodeId, NodeId, &str)]) -> Vec<EdgeId> {
        let ids = self.inner.batch_create_edges(edges);
        let epoch = self.inner.current_epoch();
        for (id, (src, dst, edge_type)) in ids.iter().zip(edges) {
            let mut event = make_event(
                EntityId::Edge(*id),
                ChangeKind::Create,
                epoch,
                self.next_ts(),
            );
            event.edge_type = Some((*edge_type).to_string());
            event.src_id = Some(src.as_u64());
            event.dst_id = Some(dst.as_u64());
            self.record_directly(event);
        }
        ids
    }

    // --- Deletion ---

    fn delete_node(&self, id: NodeId) -> bool {
        let before_props = self.collect_node_properties(id);
        let deleted = self.inner.delete_node(id);
        if deleted {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Node(id),
                ChangeKind::Delete,
                epoch,
                self.next_ts(),
            );
            event.before = before_props;
            self.record_directly(event);
        }
        deleted
    }

    fn delete_node_versioned(
        &self,
        id: NodeId,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> bool {
        let before_props = self.collect_node_properties(id);
        let labels = self.collect_node_labels(id);
        let deleted = self.inner.delete_node_versioned(id, epoch, transaction_id);
        if deleted {
            let mut event = make_event(
                EntityId::Node(id),
                ChangeKind::Delete,
                epoch,
                self.next_ts(),
            );
            event.before = before_props;
            event.labels = labels;
            self.buffer_event(event);
        }
        deleted
    }

    fn delete_node_edges(&self, node_id: NodeId) {
        // Collect edge info before deletion
        let outgoing: Vec<(NodeId, EdgeId)> = self.inner.edges_from(node_id, Direction::Outgoing);
        let incoming: Vec<(NodeId, EdgeId)> = self.inner.edges_from(node_id, Direction::Incoming);

        let edge_infos: Vec<(EdgeId, Option<HashMap<String, Value>>)> = outgoing
            .iter()
            .chain(incoming.iter())
            .map(|(_, eid)| (*eid, self.collect_edge_properties(*eid)))
            .collect();

        self.inner.delete_node_edges(node_id);

        let epoch = self.inner.current_epoch();
        for (eid, props) in edge_infos {
            let mut event = make_event(
                EntityId::Edge(eid),
                ChangeKind::Delete,
                epoch,
                self.next_ts(),
            );
            event.before = props;
            self.record_directly(event);
        }
    }

    fn delete_edge(&self, id: EdgeId) -> bool {
        let before_props = self.collect_edge_properties(id);
        let deleted = self.inner.delete_edge(id);
        if deleted {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Edge(id),
                ChangeKind::Delete,
                epoch,
                self.next_ts(),
            );
            event.before = before_props;
            self.record_directly(event);
        }
        deleted
    }

    fn delete_edge_versioned(
        &self,
        id: EdgeId,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> bool {
        let before_props = self.collect_edge_properties(id);
        let deleted = self.inner.delete_edge_versioned(id, epoch, transaction_id);
        if deleted {
            let mut event = make_event(
                EntityId::Edge(id),
                ChangeKind::Delete,
                epoch,
                self.next_ts(),
            );
            event.before = before_props;
            self.buffer_event(event);
        }
        deleted
    }

    // --- Property mutation ---

    fn set_node_property(&self, id: NodeId, key: &str, value: Value) {
        let old_value = self.inner.get_node_property(id, &PropertyKey::new(key));
        self.inner.set_node_property(id, key, value.clone());
        let epoch = self.inner.current_epoch();
        let mut event = make_event(
            EntityId::Node(id),
            ChangeKind::Update,
            epoch,
            self.next_ts(),
        );
        event.before = old_value.map(|v| {
            let mut m = HashMap::new();
            m.insert(key.to_string(), v);
            m
        });
        let mut after = HashMap::new();
        after.insert(key.to_string(), value);
        event.after = Some(after);
        self.record_directly(event);
    }

    fn set_edge_property(&self, id: EdgeId, key: &str, value: Value) {
        let old_value = self.inner.get_edge_property(id, &PropertyKey::new(key));
        self.inner.set_edge_property(id, key, value.clone());
        let epoch = self.inner.current_epoch();
        let mut event = make_event(
            EntityId::Edge(id),
            ChangeKind::Update,
            epoch,
            self.next_ts(),
        );
        event.before = old_value.map(|v| {
            let mut m = HashMap::new();
            m.insert(key.to_string(), v);
            m
        });
        let mut after = HashMap::new();
        after.insert(key.to_string(), value);
        event.after = Some(after);
        self.record_directly(event);
    }

    fn set_node_property_versioned(
        &self,
        id: NodeId,
        key: &str,
        value: Value,
        transaction_id: TransactionId,
    ) {
        let old_value = self.inner.get_node_property(id, &PropertyKey::new(key));
        self.inner
            .set_node_property_versioned(id, key, value.clone(), transaction_id);
        let epoch = self.inner.current_epoch();
        let mut event = make_event(
            EntityId::Node(id),
            ChangeKind::Update,
            epoch,
            self.next_ts(),
        );
        event.before = old_value.map(|v| {
            let mut m = HashMap::new();
            m.insert(key.to_string(), v);
            m
        });
        let mut after = HashMap::new();
        after.insert(key.to_string(), value);
        event.after = Some(after);
        self.buffer_event(event);
    }

    fn set_edge_property_versioned(
        &self,
        id: EdgeId,
        key: &str,
        value: Value,
        transaction_id: TransactionId,
    ) {
        let old_value = self.inner.get_edge_property(id, &PropertyKey::new(key));
        self.inner
            .set_edge_property_versioned(id, key, value.clone(), transaction_id);
        let epoch = self.inner.current_epoch();
        let mut event = make_event(
            EntityId::Edge(id),
            ChangeKind::Update,
            epoch,
            self.next_ts(),
        );
        event.before = old_value.map(|v| {
            let mut m = HashMap::new();
            m.insert(key.to_string(), v);
            m
        });
        let mut after = HashMap::new();
        after.insert(key.to_string(), value);
        event.after = Some(after);
        self.buffer_event(event);
    }

    fn remove_node_property(&self, id: NodeId, key: &str) -> Option<Value> {
        let removed = self.inner.remove_node_property(id, key);
        if let Some(ref old_val) = removed {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Node(id),
                ChangeKind::Update,
                epoch,
                self.next_ts(),
            );
            let mut before = HashMap::new();
            before.insert(key.to_string(), old_val.clone());
            event.before = Some(before);
            self.record_directly(event);
        }
        removed
    }

    fn remove_edge_property(&self, id: EdgeId, key: &str) -> Option<Value> {
        let removed = self.inner.remove_edge_property(id, key);
        if let Some(ref old_val) = removed {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Edge(id),
                ChangeKind::Update,
                epoch,
                self.next_ts(),
            );
            let mut before = HashMap::new();
            before.insert(key.to_string(), old_val.clone());
            event.before = Some(before);
            self.record_directly(event);
        }
        removed
    }

    fn remove_node_property_versioned(
        &self,
        id: NodeId,
        key: &str,
        transaction_id: TransactionId,
    ) -> Option<Value> {
        let removed = self
            .inner
            .remove_node_property_versioned(id, key, transaction_id);
        if let Some(ref old_val) = removed {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Node(id),
                ChangeKind::Update,
                epoch,
                self.next_ts(),
            );
            let mut before = HashMap::new();
            before.insert(key.to_string(), old_val.clone());
            event.before = Some(before);
            self.buffer_event(event);
        }
        removed
    }

    fn remove_edge_property_versioned(
        &self,
        id: EdgeId,
        key: &str,
        transaction_id: TransactionId,
    ) -> Option<Value> {
        let removed = self
            .inner
            .remove_edge_property_versioned(id, key, transaction_id);
        if let Some(ref old_val) = removed {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Edge(id),
                ChangeKind::Update,
                epoch,
                self.next_ts(),
            );
            let mut before = HashMap::new();
            before.insert(key.to_string(), old_val.clone());
            event.before = Some(before);
            self.buffer_event(event);
        }
        removed
    }

    // --- Label mutation ---

    fn add_label(&self, node_id: NodeId, label: &str) -> bool {
        let added = self.inner.add_label(node_id, label);
        if added {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Node(node_id),
                ChangeKind::Update,
                epoch,
                self.next_ts(),
            );
            event.labels = self.collect_node_labels(node_id);
            self.record_directly(event);
        }
        added
    }

    fn remove_label(&self, node_id: NodeId, label: &str) -> bool {
        let old_labels = self.collect_node_labels(node_id);
        let removed = self.inner.remove_label(node_id, label);
        if removed {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Node(node_id),
                ChangeKind::Update,
                epoch,
                self.next_ts(),
            );
            event.labels = old_labels;
            self.record_directly(event);
        }
        removed
    }

    fn add_label_versioned(
        &self,
        node_id: NodeId,
        label: &str,
        transaction_id: TransactionId,
    ) -> bool {
        let added = self
            .inner
            .add_label_versioned(node_id, label, transaction_id);
        if added {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Node(node_id),
                ChangeKind::Update,
                epoch,
                self.next_ts(),
            );
            event.labels = self.collect_node_labels(node_id);
            self.buffer_event(event);
        }
        added
    }

    fn remove_label_versioned(
        &self,
        node_id: NodeId,
        label: &str,
        transaction_id: TransactionId,
    ) -> bool {
        let old_labels = self.collect_node_labels(node_id);
        let removed = self
            .inner
            .remove_label_versioned(node_id, label, transaction_id);
        if removed {
            let epoch = self.inner.current_epoch();
            let mut event = make_event(
                EntityId::Node(node_id),
                ChangeKind::Update,
                epoch,
                self.next_ts(),
            );
            event.labels = old_labels;
            self.buffer_event(event);
        }
        removed
    }
}
