//! Integration tests verifying that CDC records session-driven mutations.
//!
//! Before the `CdcGraphStore` decorator, only direct CRUD API calls
//! (`db.create_node()`, `db.set_node_property()`) generated CDC events.
//! Session mutations via `session.execute("INSERT ...")` bypassed CDC entirely.
//!
//! These tests verify the decorator correctly buffers events during mutations,
//! flushes them on commit, and discards them on rollback.
//!
//! ```bash
//! cargo test --features "full" -p grafeo-engine --test cdc_session_mutations
//! ```

#![cfg(all(feature = "cdc", feature = "gql"))]

use grafeo_engine::cdc::{ChangeKind, EntityId};
use grafeo_engine::{Config, GrafeoDB};

fn db() -> GrafeoDB {
    GrafeoDB::with_config(Config::in_memory().with_cdc()).unwrap()
}

// ============================================================================
// Basic session mutations generate CDC events
// ============================================================================

#[test]
fn insert_through_session_generates_create_event() {
    let db = db();
    let session = db.session();
    session
        .execute("INSERT (:Person {name: 'Alix', age: 30})")
        .unwrap();

    // Find the node ID
    let result = session
        .execute("MATCH (n:Person {name: 'Alix'}) RETURN id(n) AS nid")
        .unwrap();
    assert_eq!(result.row_count(), 1, "Node should exist after INSERT");

    let node_id = match &result.rows[0][0] {
        grafeo_common::types::Value::Int64(id) => grafeo_common::types::NodeId::new(*id as u64),
        other => panic!("Expected Int64 node ID, got: {other:?}"),
    };

    // Check CDC recorded the creation
    let history = db.history(node_id).unwrap();
    assert!(
        !history.is_empty(),
        "CDC should record session INSERT, got 0 events"
    );
    assert!(
        history.iter().any(|e| e.kind == ChangeKind::Create),
        "Should contain a Create event for the session INSERT"
    );
}

#[test]
fn set_through_session_generates_update_event() {
    let db = db();
    let session = db.session();
    session.execute("INSERT (:Person {name: 'Alix'})").unwrap();

    let result = session
        .execute("MATCH (n:Person {name: 'Alix'}) RETURN id(n)")
        .unwrap();
    let node_id = match &result.rows[0][0] {
        grafeo_common::types::Value::Int64(id) => grafeo_common::types::NodeId::new(*id as u64),
        other => panic!("Expected Int64, got: {other:?}"),
    };

    // Now SET a property through session
    session
        .execute("MATCH (n:Person {name: 'Alix'}) SET n.city = 'Amsterdam'")
        .unwrap();

    let history = db.history(node_id).unwrap();
    let update_count = history
        .iter()
        .filter(|e| e.kind == ChangeKind::Update)
        .count();
    assert!(
        update_count >= 1,
        "Should have at least 1 Update event from SET, got {update_count}"
    );
}

#[test]
fn delete_through_session_generates_delete_event() {
    let db = db();
    let session = db.session();
    session.execute("INSERT (:Person {name: 'Alix'})").unwrap();

    let result = session
        .execute("MATCH (n:Person {name: 'Alix'}) RETURN id(n)")
        .unwrap();
    let node_id = match &result.rows[0][0] {
        grafeo_common::types::Value::Int64(id) => grafeo_common::types::NodeId::new(*id as u64),
        other => panic!("Expected Int64, got: {other:?}"),
    };

    session
        .execute("MATCH (n:Person {name: 'Alix'}) DELETE n")
        .unwrap();

    let history = db.history(node_id).unwrap();
    assert!(
        history.iter().any(|e| e.kind == ChangeKind::Delete),
        "Should contain a Delete event from session DELETE"
    );
}

// ============================================================================
// Transaction semantics: rollback discards CDC events
// ============================================================================

#[test]
fn rollback_discards_cdc_events() {
    let db = db();
    let mut session = db.session();

    session.begin_transaction().unwrap();
    session.execute("INSERT (:Person {name: 'Gus'})").unwrap();
    session.rollback().unwrap();

    // After rollback, there should be no nodes and no CDC events
    let result = session
        .execute("MATCH (n:Person) RETURN count(n) AS cnt")
        .unwrap();
    assert_eq!(
        result.rows[0][0],
        grafeo_common::types::Value::Int64(0),
        "Rolled-back node should not exist"
    );

    // Check that no CDC events leaked
    let changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();
    assert!(
        changes.is_empty(),
        "Rolled-back transaction should produce 0 CDC events, got {}",
        changes.len()
    );
}

#[test]
fn multi_statement_transaction_flushes_on_commit() {
    let db = db();
    let mut session = db.session();

    session.begin_transaction().unwrap();
    session.execute("INSERT (:Person {name: 'Alix'})").unwrap();
    session.execute("INSERT (:Person {name: 'Gus'})").unwrap();

    // Before commit: check that CDC log has no events yet
    let pre_commit_changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();
    assert!(
        pre_commit_changes.is_empty(),
        "CDC events should not appear before commit, got {}",
        pre_commit_changes.len()
    );

    session.commit().unwrap();

    // After commit: CDC log should have events
    let post_commit_changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();
    let create_count = post_commit_changes
        .iter()
        .filter(|e| e.kind == ChangeKind::Create)
        .count();
    assert!(
        create_count >= 2,
        "Should have at least 2 Create events after commit, got {create_count}"
    );
}

// ============================================================================
// Savepoint rollback truncates CDC buffer
// ============================================================================

#[test]
fn savepoint_rollback_discards_post_savepoint_events() {
    let db = db();
    let mut session = db.session();

    session.begin_transaction().unwrap();
    session.execute("INSERT (:Person {name: 'Alix'})").unwrap();
    session.execute("SAVEPOINT sp1").unwrap();
    session.execute("INSERT (:Person {name: 'Gus'})").unwrap();
    session.execute("ROLLBACK TO SAVEPOINT sp1").unwrap();
    session.commit().unwrap();

    // Only Alix should exist, Gus was rolled back
    let result = session
        .execute("MATCH (n:Person) RETURN n.name ORDER BY n.name")
        .unwrap();
    assert_eq!(
        result.row_count(),
        1,
        "Only Alix should exist after savepoint rollback"
    );

    // CDC should only have events for Alix, not Gus
    let changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();
    let create_count = changes
        .iter()
        .filter(|e| e.kind == ChangeKind::Create && matches!(e.entity_id, EntityId::Node(_)))
        .count();
    assert_eq!(
        create_count, 1,
        "Should have exactly 1 Create node event (Alix only), got {create_count}"
    );
}

// ============================================================================
// Edge creation/deletion through session
// ============================================================================

#[test]
fn edge_creation_through_session_generates_cdc() {
    let db = db();
    let session = db.session();
    session
        .execute("INSERT (:Person {name: 'Alix'})-[:KNOWS]->(:Person {name: 'Gus'})")
        .unwrap();

    let changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();

    let node_creates = changes
        .iter()
        .filter(|e| e.kind == ChangeKind::Create && matches!(e.entity_id, EntityId::Node(_)))
        .count();
    let edge_creates = changes
        .iter()
        .filter(|e| e.kind == ChangeKind::Create && matches!(e.entity_id, EntityId::Edge(_)))
        .count();

    assert!(
        node_creates >= 2,
        "Should have at least 2 node Create events, got {node_creates}"
    );
    assert!(
        edge_creates >= 1,
        "Should have at least 1 edge Create event, got {edge_creates}"
    );
}

// ============================================================================
// Auto-commit mode (single INSERT without explicit transaction)
// ============================================================================

#[test]
fn auto_commit_insert_generates_cdc() {
    let db = db();
    let session = db.session();

    // Single statement without explicit transaction uses auto-commit
    session
        .execute("INSERT (:Person {name: 'Vincent'})")
        .unwrap();

    let changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();
    assert!(
        !changes.is_empty(),
        "Auto-commit INSERT should generate CDC events"
    );
    assert!(
        changes.iter().any(|e| e.kind == ChangeKind::Create),
        "Should contain a Create event"
    );
}
