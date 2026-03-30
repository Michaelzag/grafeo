//! Comprehensive tests for the read-only CompactStore.
//!
//! Covers builder construction, GraphStore trait compliance, filtered search
//! with zone maps, statistics, introspection, and edge cases.

use arcstr::ArcStr;

use crate::graph::Direction;
use crate::graph::compact::builder::{CompactStoreBuilder, CompactStoreError};
use crate::graph::compact::id::{decode_edge_id, decode_node_id, encode_node_id};
use crate::graph::lpg::CompareOp;
use crate::graph::traits::GraphStore;
use grafeo_common::types::*;

// ---------------------------------------------------------------------------
// Shared test helpers
// ---------------------------------------------------------------------------

/// Builds a small graph with Person and City nodes, LIVES_IN and KNOWS edges.
///
/// Person nodes (5): Alix(25), Gus(30), Vincent(35), Jules(40), Mia(45)
/// City nodes   (3): Amsterdam, Berlin, Paris
/// LIVES_IN     (5): Person 0->City 0, 1->1, 2->2, 3->0, 4->1
/// KNOWS        (4): Person 0->1, 1->2, 2->3, 3->4
fn build_test_store() -> super::CompactStore {
    CompactStoreBuilder::new()
        .node_table("Person", |t| {
            t.column_bitpacked("age", &[25, 30, 35, 40, 45], 6)
                .column_dict("name", &["Alix", "Gus", "Vincent", "Jules", "Mia"])
        })
        .node_table("City", |t| {
            t.column_dict("name", &["Amsterdam", "Berlin", "Paris"])
        })
        .rel_table("LIVES_IN", "Person", "City", |r| {
            r.edges([(0, 0), (1, 1), (2, 2), (3, 0), (4, 1)])
                .backward(true)
        })
        .rel_table("KNOWS", "Person", "Person", |r| {
            r.edges([(0, 1), (1, 2), (2, 3), (3, 4)]).backward(true)
        })
        .build()
        .unwrap()
}

/// Returns the NodeId for Person at a given row offset within the Person table.
fn person_at(store: &super::CompactStore, offset: u64) -> NodeId {
    let ids = store.nodes_by_label("Person");
    ids.into_iter()
        .find(|id| {
            let (_, off) = decode_node_id(*id);
            off == offset
        })
        .expect("person at offset should exist")
}

/// Returns the NodeId for City at a given row offset within the City table.
fn city_at(store: &super::CompactStore, offset: u64) -> NodeId {
    let ids = store.nodes_by_label("City");
    ids.into_iter()
        .find(|id| {
            let (_, off) = decode_node_id(*id);
            off == offset
        })
        .expect("city at offset should exist")
}

// ===========================================================================
// A. Builder tests
// ===========================================================================

#[test]
fn test_builder_single_node_table() {
    let store = CompactStoreBuilder::new()
        .node_table("Person", |t| {
            t.column_bitpacked("age", &[25, 30], 6)
                .column_dict("name", &["Alix", "Gus"])
        })
        .build()
        .unwrap();

    assert_eq!(store.node_count(), 2);
    assert_eq!(store.edge_count(), 0);
    assert_eq!(store.nodes_by_label("Person").len(), 2);
}

#[test]
fn test_builder_multiple_node_tables() {
    let store = CompactStoreBuilder::new()
        .node_table("Person", |t| t.column_dict("name", &["Alix", "Gus"]))
        .node_table("City", |t| t.column_dict("name", &["Amsterdam", "Berlin"]))
        .build()
        .unwrap();

    assert_eq!(store.node_count(), 4);
    assert_eq!(store.nodes_by_label("Person").len(), 2);
    assert_eq!(store.nodes_by_label("City").len(), 2);
}

#[test]
fn test_builder_with_edges_and_backward() {
    let store = build_test_store();
    assert_eq!(store.node_count(), 8);
    assert_eq!(store.edge_count(), 9);
    assert!(store.has_backward_adjacency());
}

#[test]
fn test_builder_with_edge_properties() {
    let store = CompactStoreBuilder::new()
        .node_table("Person", |t| {
            t.column_dict("name", &["Alix", "Gus", "Vincent"])
        })
        .rel_table("RATED", "Person", "Person", |r| {
            r.edges([(0, 1), (1, 2)])
                .column_bitpacked("score", &[5, 8], 4)
        })
        .build()
        .unwrap();

    let alix = person_at(&store, 0);
    let outgoing = store.edges_from(alix, Direction::Outgoing);
    assert_eq!(outgoing.len(), 1);

    let (_, edge_id) = outgoing[0];
    let score = store.get_edge_property(edge_id, &PropertyKey::from("score"));
    assert_eq!(score, Some(Value::Int64(5)));
}

#[test]
fn test_builder_label_not_found_error() {
    let result = CompactStoreBuilder::new()
        .node_table("Person", |t| t.column_dict("name", &["Alix"]))
        .rel_table("LIVES_IN", "Person", "City", |r| r.edges([(0, 0)]))
        .build();

    assert!(matches!(result, Err(CompactStoreError::LabelNotFound(ref s)) if s == "City"));
}

#[test]
fn test_builder_column_length_mismatch_error() {
    let result = CompactStoreBuilder::new()
        .node_table("Person", |t| {
            t.column_bitpacked("age", &[25, 30, 35], 6)
                .column_dict("name", &["Alix", "Gus"]) // 2 != 3
        })
        .build();

    assert!(matches!(
        result,
        Err(CompactStoreError::ColumnLengthMismatch {
            expected: 3,
            got: 2
        })
    ));
}

#[test]
fn test_builder_duplicate_label_error() {
    let result = CompactStoreBuilder::new()
        .node_table("Person", |t| t.column_dict("name", &["Alix"]))
        .node_table("Person", |t| t.column_dict("name", &["Gus"]))
        .build();

    assert!(matches!(
        result,
        Err(CompactStoreError::DuplicateLabel(ref s)) if s == "Person"
    ));
}

#[test]
fn test_builder_duplicate_edge_type_error() {
    let result = CompactStoreBuilder::new()
        .node_table("Person", |t| t.column_dict("name", &["Alix", "Gus"]))
        .node_table("City", |t| t.column_dict("name", &["Amsterdam"]))
        .rel_table("LIVES_IN", "Person", "City", |r| r.edges(vec![(0, 0)]))
        .rel_table("LIVES_IN", "Person", "City", |r| r.edges(vec![(1, 0)]))
        .build();

    assert!(matches!(
        result,
        Err(CompactStoreError::DuplicateEdgeType(ref s)) if s == "LIVES_IN"
    ));
}

#[test]
fn test_builder_value_overflow_error() {
    // u64::MAX exceeds i64::MAX, should trigger ValueOverflow.
    let result = CompactStoreBuilder::new()
        .node_table("Bad", |t| t.column_bitpacked("big", &[u64::MAX], 64))
        .build();

    assert!(matches!(
        result,
        Err(CompactStoreError::ValueOverflow { .. })
    ));
}

#[test]
fn test_builder_bitmap_column() {
    let store = CompactStoreBuilder::new()
        .node_table("Person", |t| {
            t.column_dict("name", &["Alix", "Gus", "Vincent"])
                .column_bitmap("active", &[true, false, true])
        })
        .build()
        .unwrap();

    let alix = person_at(&store, 0);
    assert_eq!(
        store.get_node_property(alix, &PropertyKey::from("active")),
        Some(Value::Bool(true))
    );
    let gus = person_at(&store, 1);
    assert_eq!(
        store.get_node_property(gus, &PropertyKey::from("active")),
        Some(Value::Bool(false))
    );
}

#[test]
fn test_builder_int8_vector_column() {
    let store = CompactStoreBuilder::new()
        .node_table("Doc", |t| {
            t.column_dict("title", &["doc_a", "doc_b"])
                .column_int8_vector("embedding", vec![1, 2, 3, -1, -2, -3], 3)
        })
        .build()
        .unwrap();

    let ids = store.nodes_by_label("Doc");
    assert_eq!(ids.len(), 2);

    let first = ids
        .iter()
        .find(|id| decode_node_id(**id).1 == 0)
        .copied()
        .unwrap();
    let emb = store.get_node_property(first, &PropertyKey::from("embedding"));
    let expected: Vec<Value> = vec![Value::Int64(1), Value::Int64(2), Value::Int64(3)];
    assert_eq!(emb, Some(Value::List(std::sync::Arc::from(expected))));
}

#[test]
fn test_builder_empty_table() {
    let store = CompactStoreBuilder::new()
        .node_table("Empty", |t| t.column_dict("name", &[]))
        .build()
        .unwrap();

    assert_eq!(store.node_count(), 0);
    assert_eq!(store.nodes_by_label("Empty").len(), 0);
}

// ===========================================================================
// B. GraphStore trait compliance
// ===========================================================================

#[test]
fn test_get_node_returns_labels_and_properties() {
    let store = build_test_store();
    let alix = person_at(&store, 0);
    let node = store.get_node(alix).expect("node should exist");

    assert!(node.has_label("Person"));
    assert_eq!(
        node.properties.get(&PropertyKey::from("name")),
        Some(&Value::String(ArcStr::from("Alix")))
    );
    assert_eq!(
        node.properties.get(&PropertyKey::from("age")),
        Some(&Value::Int64(25))
    );
}

#[test]
fn test_get_node_nonexistent_returns_none() {
    let store = build_test_store();
    assert!(store.get_node(NodeId::new(999_999_999)).is_none());
}

#[test]
fn test_get_edge_returns_src_dst_type_properties() {
    let store = build_test_store();
    let alix = person_at(&store, 0);
    let outgoing = store.edges_from(alix, Direction::Outgoing);

    // Alix has 2 outgoing edges: LIVES_IN -> Amsterdam, KNOWS -> Gus
    assert_eq!(outgoing.len(), 2);

    for (target, eid) in &outgoing {
        let edge = store.get_edge(*eid).expect("edge should exist");
        assert_eq!(edge.src, alix);
        assert_eq!(edge.dst, *target);
        let etype = edge.edge_type.as_str();
        assert!(etype == "LIVES_IN" || etype == "KNOWS");
    }
}

#[test]
fn test_get_edge_nonexistent_returns_none() {
    let store = build_test_store();
    assert!(store.get_edge(EdgeId::new(999_999_999)).is_none());
}

#[test]
fn test_get_node_property_direct() {
    let store = build_test_store();
    let gus = person_at(&store, 1);
    assert_eq!(
        store.get_node_property(gus, &PropertyKey::from("age")),
        Some(Value::Int64(30))
    );
    assert_eq!(
        store.get_node_property(gus, &PropertyKey::from("name")),
        Some(Value::String(ArcStr::from("Gus")))
    );
}

#[test]
fn test_get_node_property_missing_key() {
    let store = build_test_store();
    let alix = person_at(&store, 0);
    assert_eq!(
        store.get_node_property(alix, &PropertyKey::from("nonexistent")),
        None
    );
}

#[test]
fn test_get_edge_property_direct() {
    let store = CompactStoreBuilder::new()
        .node_table("Person", |t| t.column_dict("name", &["Alix", "Gus"]))
        .rel_table("RATED", "Person", "Person", |r| {
            r.edges([(0, 1)]).column_bitpacked("weight", &[42], 8)
        })
        .build()
        .unwrap();

    let alix = person_at(&store, 0);
    let out = store.edges_from(alix, Direction::Outgoing);
    let (_, eid) = out[0];
    assert_eq!(
        store.get_edge_property(eid, &PropertyKey::from("weight")),
        Some(Value::Int64(42))
    );
    assert_eq!(
        store.get_edge_property(eid, &PropertyKey::from("missing")),
        None
    );
}

#[test]
fn test_get_node_property_batch() {
    let store = build_test_store();
    let person_ids = store.nodes_by_label("Person");
    let ages = store.get_node_property_batch(&person_ids, &PropertyKey::from("age"));
    assert_eq!(ages.len(), 5);
    for age in &ages {
        assert!(age.is_some());
    }
}

#[test]
fn test_nodes_by_label() {
    let store = build_test_store();
    let persons = store.nodes_by_label("Person");
    assert_eq!(persons.len(), 5);
    let cities = store.nodes_by_label("City");
    assert_eq!(cities.len(), 3);

    // Each node should actually exist and have the correct label.
    for nid in &persons {
        let node = store.get_node(*nid).unwrap();
        assert!(node.has_label("Person"));
    }
}

#[test]
fn test_nodes_by_label_nonexistent() {
    let store = build_test_store();
    assert!(store.nodes_by_label("Vehicle").is_empty());
}

#[test]
fn test_node_ids_returns_all_sorted() {
    let store = build_test_store();
    let ids = store.node_ids();
    assert_eq!(ids.len(), 8); // 5 persons + 3 cities

    // Verify sorted.
    for window in ids.windows(2) {
        assert!(window[0].as_u64() <= window[1].as_u64());
    }
}

#[test]
fn test_node_count_and_edge_count() {
    let store = build_test_store();
    assert_eq!(store.node_count(), 8);
    assert_eq!(store.edge_count(), 9); // 5 LIVES_IN + 4 KNOWS
}

#[test]
fn test_neighbors_outgoing() {
    let store = build_test_store();
    let alix = person_at(&store, 0);
    let neighbors = store.neighbors(alix, Direction::Outgoing);
    // Alix -> LIVES_IN Amsterdam (city offset 0), KNOWS Gus (person offset 1)
    assert_eq!(neighbors.len(), 2);

    for nid in &neighbors {
        assert!(store.get_node(*nid).is_some());
    }
}

#[test]
fn test_neighbors_incoming() {
    let store = build_test_store();
    // Gus (person offset 1) has incoming KNOWS from Alix (offset 0)
    let gus = person_at(&store, 1);
    let incoming = store.neighbors(gus, Direction::Incoming);

    // At least one incoming neighbor (Alix via KNOWS).
    // May also have incoming from LIVES_IN backward if Gus is a target there (he's not).
    assert!(
        !incoming.is_empty(),
        "Gus should have at least 1 incoming neighbor"
    );
}

#[test]
fn test_neighbors_both() {
    let store = build_test_store();
    let gus = person_at(&store, 1);
    let both = store.neighbors(gus, Direction::Both);
    let out = store.neighbors(gus, Direction::Outgoing);
    let inc = store.neighbors(gus, Direction::Incoming);
    assert_eq!(both.len(), out.len() + inc.len());
}

#[test]
fn test_edges_from_outgoing() {
    let store = build_test_store();
    let vincent = person_at(&store, 2);
    let outgoing = store.edges_from(vincent, Direction::Outgoing);
    // Vincent -> LIVES_IN Paris (city 2) + KNOWS Jules (person 3)
    assert_eq!(outgoing.len(), 2);
}

#[test]
fn test_edges_from_incoming() {
    let store = build_test_store();
    // Amsterdam (city offset 0) should have 2 incoming LIVES_IN edges:
    // Person 0 (Alix) and Person 3 (Jules).
    let amsterdam = city_at(&store, 0);
    let incoming = store.edges_from(amsterdam, Direction::Incoming);
    assert_eq!(incoming.len(), 2);
}

#[test]
fn test_out_degree_and_in_degree() {
    let store = build_test_store();
    let alix = person_at(&store, 0);
    // Alix outgoing: LIVES_IN + KNOWS = 2
    assert_eq!(store.out_degree(alix), 2);

    // Berlin (city offset 1) has in-degree 2: Person 1 (Gus) + Person 4 (Mia)
    let berlin = city_at(&store, 1);
    assert_eq!(store.in_degree(berlin), 2);
}

#[test]
fn test_edge_type_returns_correct_string() {
    let store = build_test_store();
    let alix = person_at(&store, 0);
    let outgoing = store.edges_from(alix, Direction::Outgoing);

    let mut types: Vec<String> = outgoing
        .iter()
        .filter_map(|(_, eid)| store.edge_type(*eid).map(|s| s.to_string()))
        .collect();
    types.sort();
    assert_eq!(types, vec!["KNOWS", "LIVES_IN"]);
}

#[test]
fn test_has_backward_adjacency() {
    let store = build_test_store();
    assert!(store.has_backward_adjacency());

    // Store without backward CSR.
    let store_no_bwd = CompactStoreBuilder::new()
        .node_table("A", |t| t.column_bitpacked("x", &[1], 4))
        .node_table("B", |t| t.column_bitpacked("x", &[2], 4))
        .rel_table("R", "A", "B", |r| r.edges([(0, 0)]).backward(false))
        .build()
        .unwrap();
    assert!(!store_no_bwd.has_backward_adjacency());
}

// ===========================================================================
// C. Filtered search + zone maps
// ===========================================================================

#[test]
fn test_find_nodes_by_property_int() {
    let store = build_test_store();
    let results = store.find_nodes_by_property("age", &Value::Int64(35));
    assert_eq!(results.len(), 1);
    let node = store.get_node(results[0]).unwrap();
    assert_eq!(
        node.properties.get(&PropertyKey::from("name")),
        Some(&Value::String(ArcStr::from("Vincent")))
    );
}

#[test]
fn test_find_nodes_by_property_string() {
    let store = build_test_store();
    // "Amsterdam" appears in City table.
    let results = store.find_nodes_by_property("name", &Value::String(ArcStr::from("Amsterdam")));
    assert_eq!(results.len(), 1);
    let node = store.get_node(results[0]).unwrap();
    assert!(node.has_label("City"));
}

#[test]
fn test_find_nodes_by_properties_multi() {
    let store = build_test_store();
    let results = store.find_nodes_by_properties(&[
        ("name", Value::String(ArcStr::from("Alix"))),
        ("age", Value::Int64(25)),
    ]);
    assert_eq!(results.len(), 1);

    // No match: correct name but wrong age.
    let no_match = store.find_nodes_by_properties(&[
        ("name", Value::String(ArcStr::from("Alix"))),
        ("age", Value::Int64(99)),
    ]);
    assert!(no_match.is_empty());
}

#[test]
fn test_find_nodes_in_range_inclusive() {
    let store = build_test_store();
    // Ages 30..=40 inclusive: Gus(30), Vincent(35), Jules(40)
    let results = store.find_nodes_in_range(
        "age",
        Some(&Value::Int64(30)),
        Some(&Value::Int64(40)),
        true,
        true,
    );
    assert_eq!(results.len(), 3);
}

#[test]
fn test_find_nodes_in_range_exclusive() {
    let store = build_test_store();
    // Ages (30..40) exclusive on both ends: only Vincent(35)
    let results = store.find_nodes_in_range(
        "age",
        Some(&Value::Int64(30)),
        Some(&Value::Int64(40)),
        false,
        false,
    );
    assert_eq!(results.len(), 1);
}

#[test]
fn test_find_nodes_in_range_unbounded_min() {
    let store = build_test_store();
    // age <= 30: Alix(25), Gus(30)
    let results = store.find_nodes_in_range("age", None, Some(&Value::Int64(30)), true, true);
    assert_eq!(results.len(), 2);
}

#[test]
fn test_node_property_might_match_zone_map_prune() {
    // Build a store where every table has the "age" column so zone maps are
    // authoritative (no conservative fallback for tables missing the column).
    let store = CompactStoreBuilder::new()
        .node_table("Young", |t| t.column_bitpacked("age", &[10, 20], 6))
        .node_table("Old", |t| t.column_bitpacked("age", &[60, 70], 7))
        .build()
        .unwrap();

    let key = PropertyKey::from("age");
    // 100 is above both tables' max (20 and 70): should be pruned.
    assert!(!store.node_property_might_match(&key, CompareOp::Eq, &Value::Int64(100)));
    // 5 is below both tables' min (10 and 60): should be pruned.
    assert!(!store.node_property_might_match(&key, CompareOp::Eq, &Value::Int64(5)));
}

#[test]
fn test_node_property_might_match_zone_map_pass() {
    let store = build_test_store();
    let key = PropertyKey::from("age");
    // 35 is in range [25, 45], so zone map should not prune.
    assert!(store.node_property_might_match(&key, CompareOp::Eq, &Value::Int64(35)));
    // Greater than 20: max(45) > 20, possible.
    assert!(store.node_property_might_match(&key, CompareOp::Gt, &Value::Int64(20)));
}

// ===========================================================================
// D. Statistics + introspection
// ===========================================================================

#[test]
fn test_statistics_node_and_edge_counts() {
    let store = build_test_store();
    let stats = store.statistics();
    assert_eq!(stats.total_nodes, 8);
    assert_eq!(stats.total_edges, 9);
}

#[test]
fn test_estimate_label_cardinality() {
    let store = build_test_store();
    assert_eq!(store.estimate_label_cardinality("Person"), 5.0);
    assert_eq!(store.estimate_label_cardinality("City"), 3.0);
    assert_eq!(store.estimate_label_cardinality("Unknown"), 0.0);
}

#[test]
fn test_estimate_avg_degree() {
    let store = build_test_store();
    // LIVES_IN: 5 edges, 5 source Person nodes -> avg out-degree = 1.0
    let avg_out = store.estimate_avg_degree("LIVES_IN", true);
    assert!((avg_out - 1.0).abs() < f64::EPSILON);

    // LIVES_IN: 5 edges, 3 target City nodes -> avg in-degree ~ 1.667
    let avg_in = store.estimate_avg_degree("LIVES_IN", false);
    assert!((avg_in - 5.0 / 3.0).abs() < 0.01);

    // Unknown edge type -> 0.0
    assert_eq!(store.estimate_avg_degree("NONEXISTENT", true), 0.0);
}

#[test]
fn test_all_labels() {
    let store = build_test_store();
    let mut labels = store.all_labels();
    labels.sort();
    assert_eq!(labels, vec!["City", "Person"]);
}

#[test]
fn test_all_edge_types() {
    let store = build_test_store();
    let mut types = store.all_edge_types();
    types.sort();
    assert_eq!(types, vec!["KNOWS", "LIVES_IN"]);
}

// ===========================================================================
// E. Edge cases
// ===========================================================================

#[test]
fn test_versioned_methods_delegate() {
    let store = build_test_store();
    let alix = person_at(&store, 0);

    // Versioned lookups should return the same result as non-versioned (read-only store).
    let node = store.get_node(alix).unwrap();
    let versioned = store
        .get_node_versioned(alix, EpochId(5), TransactionId(100))
        .unwrap();
    assert_eq!(node.id, versioned.id);
    assert_eq!(node.labels, versioned.labels);
    // Compare property values individually (PropertyMap order may differ).
    let age_key = PropertyKey::from("age");
    let name_key = PropertyKey::from("name");
    assert_eq!(
        node.properties.get(&age_key),
        versioned.properties.get(&age_key)
    );
    assert_eq!(
        node.properties.get(&name_key),
        versioned.properties.get(&name_key)
    );

    // Edge versioned lookup.
    let outgoing = store.edges_from(alix, Direction::Outgoing);
    let (_, eid) = outgoing[0];
    let edge = store.get_edge(eid).unwrap();
    let edge_v = store
        .get_edge_versioned(eid, EpochId(10), TransactionId(50))
        .unwrap();
    assert_eq!(edge.src, edge_v.src);
    assert_eq!(edge.dst, edge_v.dst);
    assert_eq!(edge.edge_type, edge_v.edge_type);
}

#[test]
fn test_current_epoch_returns_1() {
    let store = build_test_store();
    assert_eq!(store.current_epoch(), EpochId(1));
}

#[test]
fn test_memory_bytes_is_nonzero() {
    let store = build_test_store();
    assert!(
        store.memory_bytes() > 0,
        "non-empty store should report > 0 bytes"
    );
}

#[test]
fn test_node_ids_encode_correctly() {
    let store = build_test_store();
    let all_ids = store.node_ids();
    for nid in &all_ids {
        let (table_id, offset) = decode_node_id(*nid);
        // Round-trip: re-encoding should produce the same NodeId.
        let re_encoded = encode_node_id(table_id, offset);
        assert_eq!(*nid, re_encoded);
        // Table ID should be valid for our 2-table store.
        assert!(table_id < 2, "table_id should be 0 or 1, got {table_id}");
    }

    // Also verify edge ID round-trips.
    let alix = person_at(&store, 0);
    let edges = store.edges_from(alix, Direction::Outgoing);
    for (_, eid) in &edges {
        let (rel_id, pos) = decode_edge_id(*eid);
        assert!(rel_id < 2, "rel_table_id should be 0 or 1, got {rel_id}");
        // Position should be within edge count for that table.
        assert!(pos < 5, "csr_pos should be < 5, got {pos}");
    }
}

#[test]
fn test_empty_store() {
    let store = CompactStoreBuilder::new().build().unwrap();
    assert_eq!(store.node_count(), 0);
    assert_eq!(store.edge_count(), 0);
    assert!(store.node_ids().is_empty());
    assert!(store.all_labels().is_empty());
    assert!(store.all_edge_types().is_empty());
    assert!(store.get_node(NodeId::new(0)).is_none());
    assert!(store.get_edge(EdgeId::new(0)).is_none());
    assert_eq!(store.current_epoch(), EpochId(1));
    assert_eq!(store.memory_bytes(), 0);

    let stats = store.statistics();
    assert_eq!(stats.total_nodes, 0);
    assert_eq!(stats.total_edges, 0);
}

// ===========================================================================
// Additional coverage: all_property_keys, edge_property_might_match, etc.
// ===========================================================================

#[test]
fn test_all_property_keys() {
    let store = build_test_store();
    let mut keys = store.all_property_keys();
    keys.sort();
    // Person has "age" and "name", City has "name".
    assert_eq!(keys, vec!["age", "name"]);
}

#[test]
fn test_edge_property_might_match_conservative() {
    let store = build_test_store();
    // Edge properties have no zone maps, so this should always return true.
    assert!(store.edge_property_might_match(
        &PropertyKey::from("weight"),
        CompareOp::Eq,
        &Value::Int64(999)
    ));
}

#[test]
fn test_out_degree_matches_edges_from() {
    let store = build_test_store();
    for nid in store.node_ids() {
        let degree = store.out_degree(nid);
        let edges = store.edges_from(nid, Direction::Outgoing);
        assert_eq!(degree, edges.len(), "out_degree mismatch for node {nid:?}");
    }
}

#[test]
fn test_in_degree_matches_edges_from_incoming() {
    let store = build_test_store();
    for nid in store.node_ids() {
        let degree = store.in_degree(nid);
        let edges = store.edges_from(nid, Direction::Incoming);
        assert_eq!(degree, edges.len(), "in_degree mismatch for node {nid:?}");
    }
}

#[test]
fn test_edges_from_both_combines_directions() {
    let store = build_test_store();
    let vincent = person_at(&store, 2);
    let both = store.edges_from(vincent, Direction::Both);
    let out = store.edges_from(vincent, Direction::Outgoing);
    let inc = store.edges_from(vincent, Direction::Incoming);
    assert_eq!(both.len(), out.len() + inc.len());
}

#[test]
fn test_find_nodes_by_property_no_match() {
    let store = build_test_store();
    let results = store.find_nodes_by_property("age", &Value::Int64(999));
    assert!(results.is_empty());
}

#[test]
fn test_get_node_property_batch_mixed() {
    let store = build_test_store();
    let person_ids = store.nodes_by_label("Person");
    let city_ids = store.nodes_by_label("City");

    // Batch with mixed labels: "age" exists on Person but not on City.
    let mixed: Vec<NodeId> = vec![person_ids[0], city_ids[0]];
    let results = store.get_node_property_batch(&mixed, &PropertyKey::from("age"));
    assert!(results[0].is_some());
    assert!(results[1].is_none());
}

#[test]
fn test_edge_traversal_consistency() {
    let store = build_test_store();
    // For every outgoing edge, verify get_edge returns matching src/dst/type.
    for nid in store.node_ids() {
        for (target, eid) in store.edges_from(nid, Direction::Outgoing) {
            let edge = store
                .get_edge(eid)
                .expect("edge from edges_from should exist");
            assert_eq!(edge.src, nid, "edge src should match traversal origin");
            assert_eq!(edge.dst, target, "edge dst should match traversal target");
            let etype = store.edge_type(eid).expect("edge_type should exist");
            assert_eq!(edge.edge_type, etype);
        }
    }
}

#[test]
fn test_node_property_might_match_missing_column() {
    let store = build_test_store();
    // "color" does not exist on any table, so zone map is absent.
    // Should conservatively return true (no pruning possible).
    let key = PropertyKey::from("color");
    assert!(store.node_property_might_match(&key, CompareOp::Eq, &Value::Int64(0)));
}

#[test]
fn test_find_nodes_in_range_unbounded_max() {
    let store = build_test_store();
    // age >= 40: Jules(40), Mia(45)
    let results = store.find_nodes_in_range("age", Some(&Value::Int64(40)), None, true, true);
    assert_eq!(results.len(), 2);
}

// ===========================================================================
// F. Additional CompactStore coverage
// ===========================================================================

#[test]
fn test_get_nodes_properties_selective_batch() {
    let store = build_test_store();
    let ids = store.nodes_by_label("Person");
    let selective = store.get_nodes_properties_selective_batch(&ids, &[PropertyKey::from("age")]);
    assert_eq!(selective.len(), 5);
    for map in &selective {
        assert!(map.contains_key(&PropertyKey::from("age")));
        assert!(!map.contains_key(&PropertyKey::from("name")));
    }
}

#[test]
fn test_get_edges_properties_selective_batch() {
    let store = build_test_store();
    let person_ids = store.nodes_by_label("Person");
    let edges = store.edges_from(person_ids[0], Direction::Outgoing);
    let edge_ids: Vec<EdgeId> = edges.iter().map(|(_, eid)| *eid).collect();

    let selective =
        store.get_edges_properties_selective_batch(&edge_ids, &[PropertyKey::from("nonexistent")]);
    assert_eq!(selective.len(), edge_ids.len());
    for map in &selective {
        assert!(map.is_empty());
    }
}

#[test]
fn test_get_nodes_properties_batch() {
    let store = build_test_store();
    let ids = store.nodes_by_label("Person");
    let all_props = store.get_nodes_properties_batch(&ids);
    assert_eq!(all_props.len(), 5);
    for map in &all_props {
        assert!(map.contains_key(&PropertyKey::from("age")));
        assert!(map.contains_key(&PropertyKey::from("name")));
    }
}

#[test]
fn test_find_nodes_in_range_exclusive_bounds() {
    let store = build_test_store();
    // age in (25, 45) exclusive: Gus(30), Vincent(35), Jules(40)
    let results = store.find_nodes_in_range(
        "age",
        Some(&Value::Int64(25)),
        Some(&Value::Int64(45)),
        false,
        false,
    );
    assert_eq!(results.len(), 3);
}

#[test]
fn test_edge_property_access() {
    let store = CompactStoreBuilder::new()
        .node_table("Person", |t| t.column_dict("name", &["Alix", "Gus"]))
        .rel_table("RATED", "Person", "Person", |r| {
            r.edges([(0, 1)]).column_bitpacked("score", &[9], 4)
        })
        .build()
        .unwrap();

    let alix = person_at(&store, 0);
    let edges = store.edges_from(alix, Direction::Outgoing);
    let (_, eid) = edges[0];
    assert_eq!(
        store.get_edge_property(eid, &PropertyKey::from("score")),
        Some(Value::Int64(9))
    );
    assert_eq!(
        store.get_edge_property(eid, &PropertyKey::from("nonexistent")),
        None
    );
}

#[test]
fn test_memory_bytes_nonzero() {
    let store = build_test_store();
    assert!(store.memory_bytes() > 0);
}

#[test]
fn test_value_in_range_incomparable() {
    let store = build_test_store();
    // Search with string bounds on an integer column: no matches (incomparable types)
    let results = store.find_nodes_in_range(
        "age",
        Some(&Value::String(ArcStr::from("low"))),
        Some(&Value::String(ArcStr::from("high"))),
        true,
        true,
    );
    assert!(results.is_empty());
}
