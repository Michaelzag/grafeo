//! Graph projections: read-only, filtered views of a graph store.
//!
//! A [`GraphProjection`] wraps an existing [`GraphStore`] and presents a
//! subgraph defined by a [`ProjectionSpec`]. Only nodes with matching labels
//! and edges with matching types (whose endpoints are both in the projection)
//! are visible. Everything else is filtered out transparently.
//!
//! Projections are read-only: they implement [`GraphStore`] but not
//! [`super::GraphStoreMut`].
//!
//! # Example
//!
//! ```ignore
//! let spec = ProjectionSpec::new()
//!     .with_node_labels(["Person", "City"])
//!     .with_edge_types(["LIVES_IN"]);
//! let projected = GraphProjection::new(store, spec);
//! // Only Person/City nodes and LIVES_IN edges are visible
//! ```

use std::collections::HashSet;
use std::sync::Arc;

use arcstr::ArcStr;
use grafeo_common::types::{EdgeId, EpochId, NodeId, PropertyKey, TransactionId, Value};
use grafeo_common::utils::hash::FxHashMap;

use super::Direction;
use super::lpg::{CompareOp, Edge, Node};
use super::traits::GraphStore;
use crate::statistics::Statistics;

/// Defines which nodes and edges are included in a projection.
#[derive(Debug, Clone, Default)]
pub struct ProjectionSpec {
    /// Node labels to include. Empty means all nodes.
    node_labels: HashSet<String>,
    /// Edge types to include. Empty means all edges.
    edge_types: HashSet<String>,
}

impl ProjectionSpec {
    /// Creates an empty spec (all nodes, all edges).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Restricts the projection to nodes with any of these labels.
    #[must_use]
    pub fn with_node_labels(mut self, labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.node_labels = labels.into_iter().map(Into::into).collect();
        self
    }

    /// Restricts the projection to edges with any of these types.
    #[must_use]
    pub fn with_edge_types(mut self, types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.edge_types = types.into_iter().map(Into::into).collect();
        self
    }

    /// Returns true if node labels are filtered.
    fn filters_labels(&self) -> bool {
        !self.node_labels.is_empty()
    }

    /// Returns true if edge types are filtered.
    fn filters_edge_types(&self) -> bool {
        !self.edge_types.is_empty()
    }
}

/// A read-only, filtered view of a graph store.
///
/// Delegates all reads to the inner store, filtering results by the
/// [`ProjectionSpec`]. Nodes without matching labels and edges without
/// matching types are invisible.
pub struct GraphProjection {
    inner: Arc<dyn GraphStore>,
    spec: ProjectionSpec,
}

impl GraphProjection {
    /// Creates a new projection over the given store.
    pub fn new(inner: Arc<dyn GraphStore>, spec: ProjectionSpec) -> Self {
        Self { inner, spec }
    }

    /// Returns true if a node passes the label filter.
    fn node_matches(&self, node: &Node) -> bool {
        if !self.spec.filters_labels() {
            return true;
        }
        node.labels
            .iter()
            .any(|l| self.spec.node_labels.contains(l.as_str()))
    }

    /// Returns true if a node ID passes the label filter.
    fn node_id_matches(&self, id: NodeId) -> bool {
        if !self.spec.filters_labels() {
            return true;
        }
        self.inner
            .get_node(id)
            .is_some_and(|n| self.node_matches(&n))
    }

    /// Returns true if an edge type passes the type filter.
    fn edge_type_matches(&self, edge_type: &str) -> bool {
        if !self.spec.filters_edge_types() {
            return true;
        }
        self.spec.edge_types.contains(edge_type)
    }

    /// Returns true if an edge passes both endpoint and type filters.
    fn edge_matches(&self, edge: &Edge) -> bool {
        if !self.edge_type_matches(&edge.edge_type) {
            return false;
        }
        self.node_id_matches(edge.src) && self.node_id_matches(edge.dst)
    }
}

impl GraphStore for GraphProjection {
    // --- Point lookups ---

    fn get_node(&self, id: NodeId) -> Option<Node> {
        self.inner.get_node(id).filter(|n| self.node_matches(n))
    }

    fn get_edge(&self, id: EdgeId) -> Option<Edge> {
        self.inner.get_edge(id).filter(|e| self.edge_matches(e))
    }

    fn get_node_versioned(
        &self,
        id: NodeId,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> Option<Node> {
        self.inner
            .get_node_versioned(id, epoch, transaction_id)
            .filter(|n| self.node_matches(n))
    }

    /// Returns a versioned edge if it passes projection filters.
    ///
    /// **Limitation**: `edge_matches` checks endpoint visibility via `get_node`
    /// (current snapshot), not `get_node_versioned`, because `GraphProjection`
    /// does not store epoch/transaction context. This means endpoint filtering
    /// may reflect the current state rather than the requested version.
    fn get_edge_versioned(
        &self,
        id: EdgeId,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> Option<Edge> {
        self.inner
            .get_edge_versioned(id, epoch, transaction_id)
            .filter(|e| self.edge_matches(e))
    }

    fn get_node_at_epoch(&self, id: NodeId, epoch: EpochId) -> Option<Node> {
        self.inner
            .get_node_at_epoch(id, epoch)
            .filter(|n| self.node_matches(n))
    }

    fn get_edge_at_epoch(&self, id: EdgeId, epoch: EpochId) -> Option<Edge> {
        self.inner
            .get_edge_at_epoch(id, epoch)
            .filter(|e| self.edge_matches(e))
    }

    // --- Property access ---

    fn get_node_property(&self, id: NodeId, key: &PropertyKey) -> Option<Value> {
        if !self.node_id_matches(id) {
            return None;
        }
        self.inner.get_node_property(id, key)
    }

    fn get_edge_property(&self, id: EdgeId, key: &PropertyKey) -> Option<Value> {
        self.inner
            .get_edge(id)
            .filter(|e| self.edge_matches(e))
            .and_then(|_| self.inner.get_edge_property(id, key))
    }

    fn get_node_property_batch(&self, ids: &[NodeId], key: &PropertyKey) -> Vec<Option<Value>> {
        let filtered: Vec<_> = ids
            .iter()
            .map(|&id| {
                if self.node_id_matches(id) {
                    self.inner.get_node_property(id, key)
                } else {
                    None
                }
            })
            .collect();
        filtered
    }

    fn get_nodes_properties_batch(&self, ids: &[NodeId]) -> Vec<FxHashMap<PropertyKey, Value>> {
        ids.iter()
            .map(|&id| {
                if self.node_id_matches(id) {
                    self.inner
                        .get_nodes_properties_batch(std::slice::from_ref(&id))
                        .into_iter()
                        .next()
                        .unwrap_or_default()
                } else {
                    FxHashMap::default()
                }
            })
            .collect()
    }

    fn get_nodes_properties_selective_batch(
        &self,
        ids: &[NodeId],
        keys: &[PropertyKey],
    ) -> Vec<FxHashMap<PropertyKey, Value>> {
        ids.iter()
            .map(|&id| {
                if self.node_id_matches(id) {
                    self.inner
                        .get_nodes_properties_selective_batch(std::slice::from_ref(&id), keys)
                        .into_iter()
                        .next()
                        .unwrap_or_default()
                } else {
                    FxHashMap::default()
                }
            })
            .collect()
    }

    fn get_edges_properties_selective_batch(
        &self,
        ids: &[EdgeId],
        keys: &[PropertyKey],
    ) -> Vec<FxHashMap<PropertyKey, Value>> {
        ids.iter()
            .map(|&id| {
                if self.get_edge(id).is_some() {
                    self.inner
                        .get_edges_properties_selective_batch(std::slice::from_ref(&id), keys)
                        .into_iter()
                        .next()
                        .unwrap_or_default()
                } else {
                    FxHashMap::default()
                }
            })
            .collect()
    }

    // --- Traversal ---

    fn neighbors(&self, node: NodeId, direction: Direction) -> Vec<NodeId> {
        if !self.node_id_matches(node) {
            return Vec::new();
        }
        // Use edges_from (which filters by edge type and endpoint visibility)
        // and extract the target node IDs, so neighbors connected only via
        // excluded edge types are not returned.
        self.edges_from(node, direction)
            .into_iter()
            .map(|(target, _)| target)
            .collect()
    }

    fn edges_from(&self, node: NodeId, direction: Direction) -> Vec<(NodeId, EdgeId)> {
        if !self.node_id_matches(node) {
            return Vec::new();
        }
        self.inner
            .edges_from(node, direction)
            .into_iter()
            .filter(|&(target, edge_id)| {
                self.node_id_matches(target)
                    && self
                        .inner
                        .edge_type(edge_id)
                        .is_some_and(|t| self.edge_type_matches(&t))
            })
            .collect()
    }

    fn out_degree(&self, node: NodeId) -> usize {
        self.edges_from(node, Direction::Outgoing).len()
    }

    fn in_degree(&self, node: NodeId) -> usize {
        self.edges_from(node, Direction::Incoming).len()
    }

    fn has_backward_adjacency(&self) -> bool {
        self.inner.has_backward_adjacency()
    }

    // --- Scans ---

    fn node_ids(&self) -> Vec<NodeId> {
        if !self.spec.filters_labels() {
            return self.inner.node_ids();
        }
        self.inner
            .node_ids()
            .into_iter()
            .filter(|&id| self.node_id_matches(id))
            .collect()
    }

    fn all_node_ids(&self) -> Vec<NodeId> {
        if !self.spec.filters_labels() {
            return self.inner.all_node_ids();
        }
        self.inner
            .all_node_ids()
            .into_iter()
            .filter(|&id| self.node_id_matches(id))
            .collect()
    }

    fn nodes_by_label(&self, label: &str) -> Vec<NodeId> {
        if self.spec.filters_labels() && !self.spec.node_labels.contains(label) {
            return Vec::new();
        }
        self.inner.nodes_by_label(label)
    }

    fn node_count(&self) -> usize {
        self.node_ids().len()
    }

    fn edge_count(&self) -> usize {
        // Approximate: count edges whose type is in the spec
        if !self.spec.filters_edge_types() && !self.spec.filters_labels() {
            return self.inner.edge_count();
        }
        // Fallback: scan all nodes and count projected edges
        self.node_ids().iter().map(|&id| self.out_degree(id)).sum()
    }

    // --- Entity metadata ---

    fn edge_type(&self, id: EdgeId) -> Option<ArcStr> {
        // Must check both the type filter and endpoint visibility,
        // consistent with get_edge which uses edge_matches.
        let edge = self.inner.get_edge(id)?;
        if self.edge_matches(&edge) {
            Some(edge.edge_type)
        } else {
            None
        }
    }

    /// Returns the type of a versioned edge if it passes projection filters.
    ///
    /// **Limitation**: endpoint visibility is checked via `get_node` (current
    /// snapshot), not `get_node_versioned`. See `get_edge_versioned` for details.
    fn edge_type_versioned(
        &self,
        id: EdgeId,
        epoch: EpochId,
        transaction_id: TransactionId,
    ) -> Option<ArcStr> {
        let edge = self.inner.get_edge_versioned(id, epoch, transaction_id)?;
        if self.edge_matches(&edge) {
            Some(edge.edge_type)
        } else {
            None
        }
    }

    // --- Index introspection ---

    fn has_property_index(&self, property: &str) -> bool {
        self.inner.has_property_index(property)
    }

    // --- Filtered search ---

    fn find_nodes_by_property(&self, property: &str, value: &Value) -> Vec<NodeId> {
        self.inner
            .find_nodes_by_property(property, value)
            .into_iter()
            .filter(|&id| self.node_id_matches(id))
            .collect()
    }

    fn find_nodes_by_properties(&self, conditions: &[(&str, Value)]) -> Vec<NodeId> {
        self.inner
            .find_nodes_by_properties(conditions)
            .into_iter()
            .filter(|&id| self.node_id_matches(id))
            .collect()
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
            .into_iter()
            .filter(|&id| self.node_id_matches(id))
            .collect()
    }

    // --- Zone maps ---

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

    // --- Statistics ---

    fn statistics(&self) -> Arc<Statistics> {
        self.inner.statistics()
    }

    fn estimate_label_cardinality(&self, label: &str) -> f64 {
        if self.spec.filters_labels() && !self.spec.node_labels.contains(label) {
            return 0.0;
        }
        self.inner.estimate_label_cardinality(label)
    }

    fn estimate_avg_degree(&self, edge_type: &str, outgoing: bool) -> f64 {
        if self.spec.filters_edge_types() && !self.spec.edge_types.contains(edge_type) {
            return 0.0;
        }
        self.inner.estimate_avg_degree(edge_type, outgoing)
    }

    // --- Epoch ---

    fn current_epoch(&self) -> EpochId {
        self.inner.current_epoch()
    }

    // --- Schema introspection ---

    fn all_labels(&self) -> Vec<String> {
        if self.spec.filters_labels() {
            self.spec.node_labels.iter().cloned().collect()
        } else {
            self.inner.all_labels()
        }
    }

    fn all_edge_types(&self) -> Vec<String> {
        if self.spec.filters_edge_types() {
            self.spec.edge_types.iter().cloned().collect()
        } else {
            self.inner.all_edge_types()
        }
    }

    fn all_property_keys(&self) -> Vec<String> {
        self.inner.all_property_keys()
    }
}

#[cfg(test)]
#[cfg(feature = "lpg")]
mod tests {
    use super::*;
    use crate::graph::lpg::LpgStore;

    fn setup_social_graph() -> Arc<LpgStore> {
        let store = Arc::new(LpgStore::new().unwrap());
        let alix = store.create_node(&["Person"]);
        let gus = store.create_node(&["Person"]);
        let amsterdam = store.create_node(&["City"]);
        let grafeo = store.create_node(&["Software"]);

        store.set_node_property(alix, "name", Value::from("Alix"));
        store.set_node_property(gus, "name", Value::from("Gus"));
        store.set_node_property(amsterdam, "name", Value::from("Amsterdam"));
        store.set_node_property(grafeo, "name", Value::from("Grafeo"));

        store.create_edge(alix, gus, "KNOWS");
        store.create_edge(alix, amsterdam, "LIVES_IN");
        store.create_edge(gus, amsterdam, "LIVES_IN");
        store.create_edge(alix, grafeo, "CONTRIBUTES_TO");

        store
    }

    #[test]
    fn unfiltered_projection_sees_everything() {
        let store = setup_social_graph();
        let proj = GraphProjection::new(store.clone(), ProjectionSpec::new());
        assert_eq!(proj.node_count(), store.node_count());
        assert_eq!(proj.edge_count(), store.edge_count());
    }

    #[test]
    fn filter_by_label() {
        let store = setup_social_graph();
        let spec = ProjectionSpec::new().with_node_labels(["Person"]);
        let proj = GraphProjection::new(store, spec);

        assert_eq!(proj.node_count(), 2);
        assert_eq!(proj.nodes_by_label("Person").len(), 2);
        assert!(proj.nodes_by_label("City").is_empty());
        assert!(proj.nodes_by_label("Software").is_empty());
    }

    #[test]
    fn filter_by_edge_type() {
        let store = setup_social_graph();
        let spec = ProjectionSpec::new().with_edge_types(["KNOWS"]);
        let proj = GraphProjection::new(store, spec);

        // All nodes visible (no label filter), but only KNOWS edges
        assert_eq!(proj.node_count(), 4);
        assert_eq!(proj.edge_count(), 1);
    }

    #[test]
    fn combined_label_and_edge_filter() {
        let store = setup_social_graph();
        let spec = ProjectionSpec::new()
            .with_node_labels(["Person", "City"])
            .with_edge_types(["LIVES_IN"]);
        let proj = GraphProjection::new(store, spec);

        assert_eq!(proj.node_count(), 3); // 2 Person + 1 City
        assert_eq!(proj.edge_count(), 2); // 2 LIVES_IN edges
    }

    #[test]
    fn edge_excluded_when_endpoint_excluded() {
        let store = setup_social_graph();
        // Only Person nodes, but LIVES_IN edge type
        // LIVES_IN goes Person -> City, but City is excluded
        let spec = ProjectionSpec::new()
            .with_node_labels(["Person"])
            .with_edge_types(["LIVES_IN"]);
        let proj = GraphProjection::new(store, spec);

        assert_eq!(proj.node_count(), 2);
        // LIVES_IN edges should be excluded because City endpoints are filtered out
        assert_eq!(proj.edge_count(), 0);
    }

    #[test]
    fn get_node_filtered() {
        let store = setup_social_graph();
        let all_ids = store.node_ids();
        let spec = ProjectionSpec::new().with_node_labels(["Person"]);
        let proj = GraphProjection::new(store.clone(), spec);

        // Person nodes visible
        assert!(proj.get_node(all_ids[0]).is_some()); // Alix (Person)
        assert!(proj.get_node(all_ids[1]).is_some()); // Gus (Person)
        // City and Software nodes hidden
        assert!(proj.get_node(all_ids[2]).is_none()); // Amsterdam (City)
        assert!(proj.get_node(all_ids[3]).is_none()); // Grafeo (Software)
    }

    #[test]
    fn neighbors_filtered() {
        let store = setup_social_graph();
        let alix_id = store.node_ids()[0];

        // Without projection: Alix has 3 outgoing neighbors (Gus, Amsterdam, Grafeo)
        let all_neighbors: Vec<_> = store.neighbors(alix_id, Direction::Outgoing).collect();
        assert_eq!(all_neighbors.len(), 3);

        // With Person-only projection: Alix -> Gus only
        let spec = ProjectionSpec::new().with_node_labels(["Person"]);
        let proj = GraphProjection::new(store, spec);
        let neighbors = proj.neighbors(alix_id, Direction::Outgoing);
        assert_eq!(neighbors.len(), 1);
    }

    #[test]
    fn neighbors_filtered_by_edge_type() {
        let store = setup_social_graph();
        let alix_id = store.node_ids()[0];

        // With edge-type filter: only KNOWS edges visible
        // Alix KNOWS Gus, but LIVES_IN Amsterdam and CONTRIBUTES_TO Grafeo are excluded
        let spec = ProjectionSpec::new().with_edge_types(["KNOWS"]);
        let proj = GraphProjection::new(store, spec);
        let neighbors = proj.neighbors(alix_id, Direction::Outgoing);
        assert_eq!(neighbors.len(), 1);
    }

    #[test]
    fn property_access_respects_filter() {
        let store = setup_social_graph();
        let city_id = store.node_ids()[2]; // Amsterdam
        let spec = ProjectionSpec::new().with_node_labels(["Person"]);
        let proj = GraphProjection::new(store, spec);

        // City node properties are inaccessible
        assert!(
            proj.get_node_property(city_id, &PropertyKey::from("name"))
                .is_none()
        );
    }

    #[test]
    fn cardinality_estimation_respects_filter() {
        let store = setup_social_graph();
        let spec = ProjectionSpec::new()
            .with_node_labels(["Person"])
            .with_edge_types(["KNOWS"]);
        let proj = GraphProjection::new(store, spec);

        assert!(proj.estimate_label_cardinality("City") == 0.0);
        assert!(proj.estimate_avg_degree("LIVES_IN", true) == 0.0);
    }

    #[test]
    fn schema_introspection_reflects_filter() {
        let store = setup_social_graph();
        let spec = ProjectionSpec::new()
            .with_node_labels(["Person"])
            .with_edge_types(["KNOWS"]);
        let proj = GraphProjection::new(store, spec);

        let labels = proj.all_labels();
        assert_eq!(labels.len(), 1);
        assert!(labels.contains(&"Person".to_string()));

        let edge_types = proj.all_edge_types();
        assert_eq!(edge_types.len(), 1);
        assert!(edge_types.contains(&"KNOWS".to_string()));
    }
}
