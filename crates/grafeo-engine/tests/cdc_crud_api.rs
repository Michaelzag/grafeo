//! Tests for CDC recording through the direct CRUD API (`db.create_node()` etc.).
//!
//! These exercise the CDC paths in `crud.rs` that record events directly to
//! the `CdcLog` (as opposed to session-driven mutations via `CdcGraphStore`).
//!
//! ```bash
//! cargo test --features "cdc" -p grafeo-engine --test cdc_crud_api
//! ```

#![cfg(feature = "cdc")]

use grafeo_common::types::Value;
use grafeo_engine::cdc::{ChangeKind, EntityId};
use grafeo_engine::{Config, GrafeoDB};

fn db() -> GrafeoDB {
    GrafeoDB::with_config(Config::in_memory().with_cdc()).unwrap()
}

// ============================================================================
// Node creation with properties
// ============================================================================

#[test]
fn create_node_with_props_generates_cdc() {
    let db = db();
    let id = db.create_node_with_props(
        &["Person"],
        vec![("name", Value::from("Alix")), ("age", Value::Int64(30))],
    );

    let history = db.history(id).unwrap();
    assert!(!history.is_empty(), "Should have CDC events");
    let create = history
        .iter()
        .find(|e| e.kind == ChangeKind::Create)
        .unwrap();
    // The after snapshot should contain the properties
    let after = create.after.as_ref().unwrap();
    assert_eq!(after.get("name"), Some(&Value::from("Alix")));
    assert_eq!(after.get("age"), Some(&Value::Int64(30)));
}

// ============================================================================
// Node deletion with properties
// ============================================================================

#[test]
fn delete_node_generates_cdc_with_before_snapshot() {
    let db = db();
    let id = db.create_node(&["Person"]);
    db.set_node_property(id, "name", Value::from("Alix"));
    db.set_node_property(id, "city", Value::from("Amsterdam"));

    let deleted = db.delete_node(id);
    assert!(deleted);

    let history = db.history(id).unwrap();
    let del = history
        .iter()
        .find(|e| e.kind == ChangeKind::Delete)
        .unwrap();
    let before = del.before.as_ref().unwrap();
    assert_eq!(before.get("name"), Some(&Value::from("Alix")));
    assert_eq!(before.get("city"), Some(&Value::from("Amsterdam")));
}

// ============================================================================
// set_node_property with old value capture
// ============================================================================

#[test]
fn set_node_property_records_old_and_new_values() {
    let db = db();
    let id = db.create_node(&["Person"]);
    db.set_node_property(id, "name", Value::from("Alix"));
    db.set_node_property(id, "name", Value::from("Gus"));

    let history = db.history(id).unwrap();
    let updates: Vec<_> = history
        .iter()
        .filter(|e| e.kind == ChangeKind::Update)
        .collect();
    assert!(updates.len() >= 2, "Should have at least 2 Update events");

    // Second update should have before = "Alix", after = "Gus"
    let last_update = updates.last().unwrap();
    assert_eq!(
        last_update.before.as_ref().unwrap().get("name"),
        Some(&Value::from("Alix"))
    );
    assert_eq!(
        last_update.after.as_ref().unwrap().get("name"),
        Some(&Value::from("Gus"))
    );
}

// ============================================================================
// Edge creation
// ============================================================================

#[test]
fn create_edge_generates_cdc() {
    let db = db();
    let a = db.create_node(&["Person"]);
    let b = db.create_node(&["Person"]);
    let eid = db.create_edge(a, b, "KNOWS");

    let changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();

    let edge_creates: Vec<_> = changes
        .iter()
        .filter(|e| e.kind == ChangeKind::Create && e.entity_id == EntityId::Edge(eid))
        .collect();
    assert_eq!(edge_creates.len(), 1);
    assert_eq!(edge_creates[0].edge_type.as_deref(), Some("KNOWS"));
}

// ============================================================================
// Edge creation with properties
// ============================================================================

#[test]
fn create_edge_with_props_generates_cdc() {
    let db = db();
    let a = db.create_node(&["Person"]);
    let b = db.create_node(&["Person"]);
    let eid = db.create_edge_with_props(
        a,
        b,
        "KNOWS",
        vec![
            ("since", Value::Int64(2020)),
            ("weight", Value::Float64(0.8)),
        ],
    );

    let changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();

    let edge_create = changes
        .iter()
        .find(|e| e.kind == ChangeKind::Create && e.entity_id == EntityId::Edge(eid))
        .unwrap();
    let after = edge_create.after.as_ref().unwrap();
    assert_eq!(after.get("since"), Some(&Value::Int64(2020)));
    assert_eq!(after.get("weight"), Some(&Value::Float64(0.8)));
}

// ============================================================================
// Edge deletion with properties
// ============================================================================

#[test]
fn delete_edge_generates_cdc_with_before_snapshot() {
    let db = db();
    let a = db.create_node(&["Person"]);
    let b = db.create_node(&["Person"]);
    let eid = db.create_edge(a, b, "KNOWS");
    db.set_edge_property(eid, "since", Value::Int64(2020));

    let deleted = db.delete_edge(eid);
    assert!(deleted);

    let changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();

    let edge_del = changes
        .iter()
        .find(|e| e.kind == ChangeKind::Delete && e.entity_id == EntityId::Edge(eid))
        .unwrap();
    let before = edge_del.before.as_ref().unwrap();
    assert_eq!(before.get("since"), Some(&Value::Int64(2020)));
}

// ============================================================================
// set_edge_property with old value capture
// ============================================================================

#[test]
fn set_edge_property_records_old_and_new_values() {
    let db = db();
    let a = db.create_node(&["Person"]);
    let b = db.create_node(&["Person"]);
    let eid = db.create_edge(a, b, "KNOWS");
    db.set_edge_property(eid, "weight", Value::Float64(0.5));
    db.set_edge_property(eid, "weight", Value::Float64(0.9));

    let changes = db
        .changes_between(
            grafeo_common::types::EpochId::new(0),
            grafeo_common::types::EpochId::new(u64::MAX),
        )
        .unwrap();

    let edge_updates: Vec<_> = changes
        .iter()
        .filter(|e| e.kind == ChangeKind::Update && e.entity_id == EntityId::Edge(eid))
        .collect();
    assert!(edge_updates.len() >= 2);

    let last = edge_updates.last().unwrap();
    assert_eq!(
        last.before.as_ref().unwrap().get("weight"),
        Some(&Value::Float64(0.5))
    );
    assert_eq!(
        last.after.as_ref().unwrap().get("weight"),
        Some(&Value::Float64(0.9))
    );
}
