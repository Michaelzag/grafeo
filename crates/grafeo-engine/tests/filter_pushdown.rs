//! Tests for filter pushdown optimization.
//!
//! Verifies that the planner pushes equality predicates down to the store level
//! (bypassing DataChunk/expression overhead) and correctly handles:
//! - Index-based pushdown (existing behaviour)
//! - Label-first pushdown (no index, with label)
//! - Compound predicates with remaining non-equality parts
//! - Non-pushable expressions (kept as generic FilterOperator)

use grafeo_common::types::Value;
use grafeo_engine::GrafeoDB;

/// Builds a small social graph for filter tests.
///
/// 5 Person nodes (Alice/NYC, Bob/NYC, Carol/London, Dave/London, Eve/Paris)
/// 2 Company nodes (Acme, Globex)
fn setup() -> GrafeoDB {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();

    session
        .execute(
            "CREATE (a:Person {name: 'Alice', city: 'NYC', age: 30}),
                    (b:Person {name: 'Bob',   city: 'NYC', age: 25}),
                    (c:Person {name: 'Carol', city: 'London', age: 35}),
                    (d:Person {name: 'Dave',  city: 'London', age: 40}),
                    (e:Person {name: 'Eve',   city: 'Paris',  age: 28}),
                    (x:Company {name: 'Acme'}),
                    (y:Company {name: 'Globex'})",
        )
        .unwrap();

    db
}

// ── Equality with label, no index (new pushdown path) ──

#[test]
fn equality_filter_pushdown_without_index() {
    let db = setup();
    let session = db.session();

    let result = session
        .execute("MATCH (n:Person) WHERE n.name = 'Alice' RETURN n.name, n.city")
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::from("Alice"));
    assert_eq!(result.rows[0][1], Value::from("NYC"));
}

#[test]
fn compound_equality_pushdown_without_index() {
    let db = setup();
    let session = db.session();

    let result = session
        .execute("MATCH (n:Person) WHERE n.city = 'NYC' AND n.age = 25 RETURN n.name")
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::from("Bob"));
}

// ── Equality with property index (existing path still works) ──

#[test]
fn equality_filter_pushdown_with_index() {
    let db = setup();
    db.create_property_index("name");
    let session = db.session();

    let result = session
        .execute("MATCH (n:Person) WHERE n.name = 'Carol' RETURN n.city")
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::from("London"));
}

// ── Compound predicate: equality + range (remaining predicate handling) ──

#[test]
fn mixed_equality_and_range_pushdown() {
    let db = setup();
    let session = db.session();

    // Equality on city pushed down, range on age kept as FilterOperator
    let result = session
        .execute("MATCH (n:Person) WHERE n.city = 'London' AND n.age > 36 RETURN n.name")
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::from("Dave"));
}

#[test]
fn mixed_equality_and_range_no_match() {
    let db = setup();
    let session = db.session();

    // Equality matches Carol (35) and Dave (40), but range > 50 matches nobody
    let result = session
        .execute("MATCH (n:Person) WHERE n.city = 'London' AND n.age > 50 RETURN n.name")
        .unwrap();

    assert!(result.rows.is_empty());
}

// ── Range-only pushdown ──

#[test]
fn range_filter_pushdown() {
    let db = setup();
    let session = db.session();

    let result = session
        .execute("MATCH (n:Person) WHERE n.age > 30 RETURN n.name")
        .unwrap();

    // Carol (35) and Dave (40) match
    assert_eq!(result.rows.len(), 2);
    let names: Vec<&Value> = result.rows.iter().map(|r| &r[0]).collect();
    assert!(names.contains(&&Value::from("Carol")));
    assert!(names.contains(&&Value::from("Dave")));
}

#[test]
fn between_filter_pushdown() {
    let db = setup();
    let session = db.session();

    let result = session
        .execute("MATCH (n:Person) WHERE n.age >= 28 AND n.age <= 35 RETURN n.name")
        .unwrap();

    // Alice (30), Carol (35), Eve (28) match
    assert_eq!(result.rows.len(), 3);
    let names: Vec<&Value> = result.rows.iter().map(|r| &r[0]).collect();
    assert!(names.contains(&&Value::from("Alice")));
    assert!(names.contains(&&Value::from("Carol")));
    assert!(names.contains(&&Value::from("Eve")));
}

// ── Non-pushable expressions (stay as generic filter) ──

#[test]
fn non_pushable_expression_filter() {
    let db = setup();
    let session = db.session();

    // String function in predicate — not pushable, uses generic FilterOperator
    let result = session
        .execute("MATCH (n:Person) WHERE n.name STARTS WITH 'A' RETURN n.name")
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::from("Alice"));
}

// ── No label (no pushdown without label or index) ──

#[test]
fn equality_without_label_or_index_falls_through() {
    let db = setup();
    let session = db.session();

    // No label, no index — falls through to generic FilterOperator
    // Should still return correct results
    let result = session
        .execute("MATCH (n) WHERE n.name = 'Alice' RETURN n.name")
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::from("Alice"));
}

// ── Label selectivity: only label-matching nodes checked ──

#[test]
fn label_narrows_scan_correctly() {
    let db = setup();
    let session = db.session();

    // Company has name='Acme' too, but label restricts to Person
    let result = session
        .execute("MATCH (n:Person) WHERE n.name = 'Acme' RETURN n.name")
        .unwrap();

    assert!(result.rows.is_empty());
}

#[test]
fn label_filter_on_company() {
    let db = setup();
    let session = db.session();

    let result = session
        .execute("MATCH (n:Company) WHERE n.name = 'Acme' RETURN n.name")
        .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::from("Acme"));
}

// ── MVCC visibility: rolled-back nodes must not leak through pushdown ──

#[test]
fn rollback_hides_nodes_from_equality_pushdown() {
    let db = setup();
    let mut session = db.session();

    // Create a node inside a transaction, then roll back
    session.begin_tx().unwrap();
    session
        .execute("CREATE (:Person {name: 'Ghost', city: 'Nowhere'})")
        .unwrap();
    session.rollback().unwrap();

    // Equality pushdown on label+property must NOT return the rolled-back node
    let result = session
        .execute("MATCH (n:Person) WHERE n.name = 'Ghost' RETURN n.name")
        .unwrap();
    assert!(
        result.rows.is_empty(),
        "rolled-back node leaked through equality pushdown"
    );

    // Original nodes still visible
    let result = session
        .execute("MATCH (n:Person) WHERE n.name = 'Alice' RETURN n.name")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn rollback_hides_nodes_from_range_pushdown() {
    let db = setup();
    let mut session = db.session();

    // Create a node inside a transaction, then roll back
    session.begin_tx().unwrap();
    session
        .execute("CREATE (:Person {name: 'Ghost', city: 'Nowhere', age: 99})")
        .unwrap();
    session.rollback().unwrap();

    // Range pushdown must NOT return the rolled-back node
    let result = session
        .execute("MATCH (n:Person) WHERE n.age > 90 RETURN n.name")
        .unwrap();
    assert!(
        result.rows.is_empty(),
        "rolled-back node leaked through range pushdown"
    );
}

#[test]
fn committed_tx_nodes_visible_in_pushdown() {
    let db = setup();
    let mut session = db.session();

    // Create a node inside a transaction and commit
    session.begin_tx().unwrap();
    session
        .execute("CREATE (:Person {name: 'Frank', city: 'Berlin', age: 50})")
        .unwrap();
    session.commit().unwrap();

    // Equality pushdown should find the committed node
    let result = session
        .execute("MATCH (n:Person) WHERE n.name = 'Frank' RETURN n.city")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::from("Berlin"));

    // Range pushdown should also find it
    let result = session
        .execute("MATCH (n:Person) WHERE n.age > 45 RETURN n.name")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::from("Frank"));
}
