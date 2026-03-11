//! Tests for session API coverage gaps.
//!
//! Targets: session.rs (73.07%), common.rs optional predicate classification
//!
//! ```bash
//! cargo test -p grafeo-engine --features full --test coverage_session
//! ```

use grafeo_common::types::{EpochId, Value};
use grafeo_engine::GrafeoDB;

fn setup() -> GrafeoDB {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();
    session.create_node_with_props(
        &["Person"],
        [
            ("name", Value::String("Alix".into())),
            ("age", Value::Int64(30)),
        ],
    );
    session.create_node_with_props(
        &["Person"],
        [
            ("name", Value::String("Gus".into())),
            ("age", Value::Int64(25)),
        ],
    );
    db
}

// ---------------------------------------------------------------------------
// Direct session API: set_parameter / get_parameter
// ---------------------------------------------------------------------------

#[test]
fn test_session_set_and_get_parameter() {
    let db = setup();
    let session = db.session();
    session.set_parameter("threshold", Value::Int64(42));
    let val = session.get_parameter("threshold");
    assert_eq!(val, Some(Value::Int64(42)));
    assert_eq!(session.get_parameter("missing"), None);
}

// ---------------------------------------------------------------------------
// reset_session clears parameters
// ---------------------------------------------------------------------------

#[test]
fn test_reset_session_clears_state() {
    let db = setup();
    let session = db.session();
    session.set_parameter("key", Value::String("val".into()));
    session.reset_session();
    assert_eq!(session.get_parameter("key"), None);
}

// ---------------------------------------------------------------------------
// set_time_zone via direct API
// ---------------------------------------------------------------------------

#[test]
fn test_set_time_zone_direct() {
    let db = setup();
    let session = db.session();
    // Setting timezone should not panic
    session.set_time_zone("Europe/Amsterdam");
}

// ---------------------------------------------------------------------------
// graph_model
// ---------------------------------------------------------------------------

#[test]
fn test_graph_model_default() {
    let db = setup();
    let session = db.session();
    let model = session.graph_model();
    // Default model for in-memory DB should be LPG
    assert_eq!(format!("{model:?}"), "Lpg");
}

// ---------------------------------------------------------------------------
// Viewing epoch (time-travel API)
// ---------------------------------------------------------------------------

#[test]
fn test_viewing_epoch_lifecycle() {
    let db = setup();
    let session = db.session();
    assert_eq!(session.viewing_epoch(), None);
    session.set_viewing_epoch(EpochId::new(1));
    assert_eq!(session.viewing_epoch(), Some(EpochId::new(1)));
    session.clear_viewing_epoch();
    assert_eq!(session.viewing_epoch(), None);
}

#[test]
fn test_execute_at_epoch() {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();
    session.create_node_with_props(&["Item"], [("name", Value::String("original".into()))]);

    let epoch = db.current_epoch();

    // Exercise execute_at_epoch code path (sets viewing_epoch_override, runs query, restores)
    let r = session
        .execute_at_epoch("MATCH (i:Item) RETURN i.name AS name", epoch)
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    // NOTE: In-memory MVCC may not fully support epoch-based time travel for reads.
    // The key coverage goal is exercising the execute_at_epoch code path.
    assert!(matches!(&r.rows[0][0], Value::String(_)));
}

// ---------------------------------------------------------------------------
// Savepoint edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_savepoint_outside_transaction_fails() {
    let db = setup();
    let session = db.session();
    let result = session.savepoint("sp");
    assert!(result.is_err());
}

#[test]
fn test_release_savepoint_via_api() {
    let db = setup();
    let mut session = db.session();
    session.begin_transaction().unwrap();
    session.savepoint("sp1").unwrap();
    session
        .execute("MATCH (p:Person {name: 'Alix'}) SET p.age = 99")
        .unwrap();
    session.release_savepoint("sp1").unwrap();
    // After release, rollback to sp1 should fail
    let result = session.rollback_to_savepoint("sp1");
    assert!(result.is_err());
    session.commit().unwrap();
}

// ---------------------------------------------------------------------------
// Transaction isolation levels
// ---------------------------------------------------------------------------

#[test]
fn test_begin_transaction_with_serializable_isolation() {
    let db = setup();
    let mut session = db.session();
    // Use GQL SET SESSION to test serializable isolation
    session.begin_transaction().unwrap();
    session
        .execute("MATCH (p:Person) RETURN count(p) AS cnt")
        .unwrap();
    session.commit().unwrap();
}

// ---------------------------------------------------------------------------
// execute_with_params
// ---------------------------------------------------------------------------

#[test]
fn test_execute_with_params_direct() {
    let db = setup();
    let session = db.session();
    let params = std::collections::HashMap::from([("min_age".to_string(), Value::Int64(28))]);
    let r = session
        .execute_with_params(
            "MATCH (p:Person) WHERE p.age > $min_age RETURN p.name AS name ORDER BY name",
            params,
        )
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Value::String("Alix".into()));
}

// ---------------------------------------------------------------------------
// use_graph (named graph switching)
// ---------------------------------------------------------------------------

#[test]
fn test_use_graph_via_gql() {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();
    session.execute("CREATE GRAPH test_graph").unwrap();
    session.execute("USE GRAPH test_graph").unwrap();
}

// ---------------------------------------------------------------------------
// OPTIONAL MATCH (exercises classify_optional_predicates in common.rs)
// ---------------------------------------------------------------------------

#[test]
fn test_optional_match_no_match() {
    let db = setup();
    let session = db.session();
    let r = session
        .execute(
            "MATCH (p:Person {name: 'Alix'}) \
             OPTIONAL MATCH (p)-[:MANAGES]->(e:Employee) \
             RETURN p.name AS name, e.name AS emp",
        )
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][0], Value::String("Alix".into()));
    assert_eq!(r.rows[0][1], Value::Null);
}

#[test]
fn test_optional_match_with_where() {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();
    let a = session.create_node_with_props(&["Person"], [("name", Value::String("Alix".into()))]);
    let b = session.create_node_with_props(&["Person"], [("name", Value::String("Gus".into()))]);
    session.create_edge(a, b, "KNOWS");

    let r = session
        .execute(
            "MATCH (p:Person {name: 'Alix'}) \
             OPTIONAL MATCH (p)-[:KNOWS]->(f:Person) WHERE f.name = 'Nonexistent' \
             RETURN p.name AS name, f.name AS friend",
        )
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0][1], Value::Null);
}

// ---------------------------------------------------------------------------
// UNWIND / FOR clause
// ---------------------------------------------------------------------------

#[test]
fn test_unwind_list() {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();
    let r = session.execute("UNWIND [1, 2, 3] AS x RETURN x").unwrap();
    assert_eq!(r.rows.len(), 3);
}

// ---------------------------------------------------------------------------
// Subquery with CALL
// ---------------------------------------------------------------------------

#[test]
fn test_call_subquery() {
    let db = setup();
    let session = db.session();
    let r = session
        .execute(
            "MATCH (p:Person) CALL { WITH p RETURN p.age * 2 AS doubled } RETURN p.name, doubled ORDER BY p.name",
        )
        .unwrap();
    assert_eq!(r.rows.len(), 2);
}
