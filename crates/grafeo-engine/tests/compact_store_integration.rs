//! Integration tests for CompactStore through GrafeoDB::with_read_store() + GQL.
//!
//! Requires features: `compact-store` + `gql` (default) for query execution.
//!
//! Validates that CompactStore works end-to-end as an external read-only store:
//! queries are planned and executed against CompactStore data through the same
//! session interface as LpgStore.

#![cfg(feature = "compact-store")]

use std::sync::Arc;

use grafeo_core::graph::compact::CompactStoreBuilder;
use grafeo_core::graph::traits::GraphStore;
use grafeo_engine::{Config, GrafeoDB};

/// Build a CompactStore with test data and wrap it in GrafeoDB::with_store().
fn build_test_db() -> GrafeoDB {
    let scores: Vec<u64> = (0..10).map(|i| (i % 5) + 1).collect();
    let names: Vec<&str> = vec![
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    ];

    let ratings: Vec<u64> = (0..50).map(|i| (i % 5) + 1).collect();

    // Each of 50 activities links to one of 10 items.
    let activity_to_item: Vec<(u32, u32)> = (0..50).map(|i| (i, i % 10)).collect();

    let store = CompactStoreBuilder::new()
        .node_table("Item", |t| {
            t.column_bitpacked("score", &scores, 4)
                .column_dict("name", &names)
        })
        .node_table("Activity", |t| t.column_bitpacked("rating", &ratings, 4))
        .rel_table("ACTIVITY_ON", "Activity", "Item", |r| {
            r.edges(activity_to_item).backward(true)
        })
        .build()
        .expect("CompactStore build failed");

    GrafeoDB::with_read_store(Arc::new(store) as Arc<dyn GraphStore>, Config::default())
        .expect("GrafeoDB::with_read_store failed")
}

// ── Basic scan queries ──────────────────────────────────────────

#[test]
fn match_all_items() {
    let db = build_test_db();
    let session = db.session();
    let result = session.execute("MATCH (n:Item) RETURN n").unwrap();
    assert_eq!(result.rows.len(), 10);
}

#[test]
fn match_all_activities() {
    let db = build_test_db();
    let session = db.session();
    let result = session.execute("MATCH (n:Activity) RETURN n").unwrap();
    assert_eq!(result.rows.len(), 50);
}

// ── Property access ──────────────────────────────────────────────

#[test]
fn return_property() {
    let db = build_test_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Item) RETURN n.name ORDER BY n.name")
        .unwrap();
    assert_eq!(result.rows.len(), 10);
    // Verify we get string values back
    let first_name = &result.rows[0][0];
    assert!(
        matches!(first_name, grafeo_common::types::Value::String(_)),
        "Expected string property, got {:?}",
        first_name
    );
}

// ── Edge traversal ───────────────────────────────────────────────

#[test]
fn traverse_outgoing() {
    let db = build_test_db();
    let session = db.session();
    let result = session
        .execute("MATCH (a:Activity)-[:ACTIVITY_ON]->(i:Item) RETURN a, i")
        .unwrap();
    // 50 activities, each with one ACTIVITY_ON edge
    assert_eq!(result.rows.len(), 50);
}

#[test]
fn traverse_incoming() {
    let db = build_test_db();
    let session = db.session();
    let result = session
        .execute("MATCH (i:Item)<-[:ACTIVITY_ON]-(a:Activity) RETURN i, a")
        .unwrap();
    // Same 50 edges, traversed from the other direction
    assert_eq!(result.rows.len(), 50);
}

// ── Aggregation ──────────────────────────────────────────────────

#[test]
fn count_per_label() {
    let db = build_test_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Item) RETURN count(n) AS cnt")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], grafeo_common::types::Value::Int64(10));
}

// ── Read-only enforcement ───────────────────────────────────────

#[test]
fn create_rejected_on_read_only_store() {
    let db = build_test_db();
    let session = db.session();

    // Mutations should be rejected on a read-only store
    let result = session.execute("CREATE (:Item {name: 'lambda'})");
    assert!(result.is_err(), "CREATE should fail on a read-only store");
}

// ── GrafeoDB::compact() ────────────────────────────────────────

#[test]
fn compact_converts_lpg_to_compact() {
    let mut db = GrafeoDB::new_in_memory();

    // Insert data via GQL.
    db.execute("INSERT (:Person {name: 'Alix', age: 30})")
        .unwrap();
    db.execute("INSERT (:Person {name: 'Gus', age: 25})")
        .unwrap();
    db.execute("INSERT (:City {name: 'Amsterdam'})").unwrap();
    db.execute(
        "MATCH (p:Person {name: 'Alix'}), (c:City {name: 'Amsterdam'}) \
         INSERT (p)-[:LIVES_IN]->(c)",
    )
    .unwrap();
    db.execute(
        "MATCH (p:Person {name: 'Gus'}), (c:City {name: 'Amsterdam'}) \
         INSERT (p)-[:LIVES_IN]->(c)",
    )
    .unwrap();

    // Compact.
    db.compact().unwrap();

    // Verify read queries still work.
    let session = db.session();
    let persons = session
        .execute("MATCH (p:Person) RETURN p.name ORDER BY p.name")
        .unwrap();
    assert_eq!(persons.rows.len(), 2);

    let cities = session.execute("MATCH (c:City) RETURN c.name").unwrap();
    assert_eq!(cities.rows.len(), 1);

    // Verify edge traversal.
    let edges = session
        .execute("MATCH (p:Person)-[:LIVES_IN]->(c:City) RETURN p.name, c.name")
        .unwrap();
    assert_eq!(edges.rows.len(), 2);

    // Verify write queries fail.
    let write_result = session.execute("INSERT (:Person {name: 'Vincent'})");
    assert!(write_result.is_err(), "writes should fail after compact()");
}

#[test]
fn compact_preserves_bool_and_string_properties() {
    let mut db = GrafeoDB::new_in_memory();

    db.execute("INSERT (:Item {name: 'alpha', active: true})")
        .unwrap();
    db.execute("INSERT (:Item {name: 'beta', active: false})")
        .unwrap();

    db.compact().unwrap();

    let session = db.session();
    let result = session
        .execute("MATCH (n:Item) RETURN n.name, n.active ORDER BY n.name")
        .unwrap();
    assert_eq!(result.rows.len(), 2);

    // First row: "alpha", true
    assert_eq!(
        result.rows[0][0],
        grafeo_common::types::Value::String(arcstr::literal!("alpha"))
    );
    assert_eq!(result.rows[0][1], grafeo_common::types::Value::Bool(true));

    // Second row: "beta", false
    assert_eq!(
        result.rows[1][0],
        grafeo_common::types::Value::String(arcstr::literal!("beta"))
    );
    assert_eq!(result.rows[1][1], grafeo_common::types::Value::Bool(false));
}

#[test]
fn compact_empty_database() {
    let mut db = GrafeoDB::new_in_memory();
    db.compact().unwrap();

    let session = db.session();
    let result = session.execute("MATCH (n) RETURN count(n)").unwrap();
    assert_eq!(result.rows[0][0], grafeo_common::types::Value::Int64(0));
}
