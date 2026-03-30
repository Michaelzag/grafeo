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
