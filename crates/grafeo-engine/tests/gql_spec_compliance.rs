//! GQL spec compliance tests for features verified during the 0.5.13 audit.
//!
//! These tests validate features that were discovered to be fully working
//! during codebase exploration, plus newly implemented features.

use grafeo_common::types::Value;
use grafeo_engine::GrafeoDB;

fn setup_db() -> GrafeoDB {
    let db = GrafeoDB::new_in_memory();
    let mut session = db.session();
    session.begin_tx().unwrap();
    session
        .execute("INSERT (:Person {name: 'Alice', age: 30})")
        .unwrap();
    session
        .execute("INSERT (:Person {name: 'Bob', age: 25})")
        .unwrap();
    session
        .execute("INSERT (:Person {name: 'Charlie', age: 35})")
        .unwrap();
    session.commit().unwrap();

    // Create edges
    session.begin_tx().unwrap();
    session
        .execute(
            "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) INSERT (a)-[:KNOWS]->(b)",
        )
        .unwrap();
    session
        .execute(
            "MATCH (a:Person {name: 'Bob'}), (b:Person {name: 'Charlie'}) INSERT (a)-[:KNOWS]->(b)",
        )
        .unwrap();
    session.commit().unwrap();
    db
}

// ---------------------------------------------------------------------------
// Phase 1: Already-working features verification
// ---------------------------------------------------------------------------

#[test]
fn test_return_star() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person {name: 'Alice'}) RETURN *")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    // Should have at least the 'n' variable
    assert!(!result.columns.is_empty());
}

#[test]
fn test_with_star() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person) WITH * WHERE n.age > 28 RETURN n.name")
        .unwrap();
    assert_eq!(result.rows.len(), 2); // Alice (30) and Charlie (35)
}

#[test]
fn test_fetch_first_n_rows() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person) RETURN n.name FETCH FIRST 2 ROWS ONLY")
        .unwrap();
    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_fetch_next_n_row() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person) RETURN n.name FETCH NEXT 1 ROW ONLY")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
}

#[test]
fn test_list_comprehension() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person {name: 'Alice'}) RETURN [x IN [1, 2, 3, 4, 5] WHERE x > 2 | x * 10] AS filtered")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    // Should be [30, 40, 50]
    match &result.rows[0][0] {
        Value::List(items) => {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], Value::Int64(30));
            assert_eq!(items[1], Value::Int64(40));
            assert_eq!(items[2], Value::Int64(50));
        }
        other => panic!("Expected list, got {:?}", other),
    }
}

#[test]
fn test_list_predicate_all() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute(
            "MATCH (n:Person {name: 'Alice'}) RETURN all(x IN [2, 4, 6] WHERE x % 2 = 0) AS result",
        )
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Bool(true));
}

#[test]
fn test_list_predicate_any() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute(
            "MATCH (n:Person {name: 'Alice'}) RETURN any(x IN [1, 2, 3] WHERE x > 2) AS result",
        )
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Bool(true));
}

#[test]
fn test_list_predicate_none() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute(
            "MATCH (n:Person {name: 'Alice'}) RETURN none(x IN [1, 2, 3] WHERE x > 10) AS result",
        )
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Bool(true));
}

#[test]
fn test_list_predicate_single() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute(
            "MATCH (n:Person {name: 'Alice'}) RETURN single(x IN [1, 2, 3] WHERE x = 2) AS result",
        )
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Bool(true));
}

#[test]
fn test_except_all() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute(
            "MATCH (n:Person) RETURN n.name \
             EXCEPT ALL \
             MATCH (n:Person {name: 'Bob'}) RETURN n.name",
        )
        .unwrap();
    assert_eq!(result.rows.len(), 2); // Alice, Charlie
}

#[test]
fn test_intersect_all() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute(
            "MATCH (n:Person) RETURN n.name \
             INTERSECT ALL \
             MATCH (n:Person) WHERE n.age >= 30 RETURN n.name",
        )
        .unwrap();
    assert_eq!(result.rows.len(), 2); // Alice, Charlie
}

// ---------------------------------------------------------------------------
// Phase 2: LIKE operator
// ---------------------------------------------------------------------------

#[test]
fn test_like_percent_wildcard() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person) WHERE n.name LIKE 'A%' RETURN n.name")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::String("Alice".into()));
}

#[test]
fn test_like_underscore_wildcard() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person) WHERE n.name LIKE 'Bo_' RETURN n.name")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::String("Bob".into()));
}

#[test]
fn test_like_no_match() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person) WHERE n.name LIKE 'X%' RETURN n.name")
        .unwrap();
    assert!(result.rows.is_empty());
}

// ---------------------------------------------------------------------------
// Phase 3: Temporal type conversions
// ---------------------------------------------------------------------------

#[test]
fn test_cast_to_date() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person {name: 'Alice'}) RETURN CAST('2024-06-15' AS DATE) AS d")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    match &result.rows[0][0] {
        Value::Date(_) => {} // OK
        other => panic!("Expected Date, got {:?}", other),
    }
}

#[test]
fn test_cast_to_time() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person {name: 'Alice'}) RETURN CAST('14:30:00' AS TIME) AS t")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    match &result.rows[0][0] {
        Value::Time(_) => {} // OK
        other => panic!("Expected Time, got {:?}", other),
    }
}

#[test]
fn test_cast_to_duration() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person {name: 'Alice'}) RETURN CAST('P1Y2M3D' AS DURATION) AS dur")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    match &result.rows[0][0] {
        Value::Duration(_) => {} // OK
        other => panic!("Expected Duration, got {:?}", other),
    }
}

#[test]
fn test_todate_function() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person {name: 'Alice'}) RETURN toDate('2024-06-15') AS d")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    match &result.rows[0][0] {
        Value::Date(_) => {} // OK
        other => panic!("Expected Date, got {:?}", other),
    }
}

#[test]
fn test_totime_function() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person {name: 'Alice'}) RETURN toTime('14:30:00') AS t")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    match &result.rows[0][0] {
        Value::Time(_) => {} // OK
        other => panic!("Expected Time, got {:?}", other),
    }
}

#[test]
fn test_toduration_function() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person {name: 'Alice'}) RETURN toDuration('P1Y2M') AS dur")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    match &result.rows[0][0] {
        Value::Duration(_) => {} // OK
        other => panic!("Expected Duration, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Phase 5: NODETACH DELETE
// ---------------------------------------------------------------------------

#[test]
fn test_nodetach_delete_errors_with_edges() {
    let db = setup_db();
    let mut session = db.session();
    session.begin_tx().unwrap();
    // Alice has an outgoing KNOWS edge, so bare DELETE should error
    let result = session.execute("MATCH (n:Person {name: 'Alice'}) DELETE n");
    assert!(result.is_err(), "DELETE on node with edges should error");
    session.rollback().unwrap();
}

#[test]
fn test_detach_delete_with_edges_succeeds() {
    let db = setup_db();
    let mut session = db.session();
    session.begin_tx().unwrap();
    let result = session.execute("MATCH (n:Person {name: 'Alice'}) DETACH DELETE n");
    assert!(result.is_ok(), "DETACH DELETE should succeed");
    session.commit().unwrap();
}

// ---------------------------------------------------------------------------
// Phase 6: CALL { subquery }
// ---------------------------------------------------------------------------

#[test]
fn test_call_inline_subquery() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute("MATCH (n:Person) CALL { MATCH (m:Person) RETURN count(m) AS total } RETURN n.name, total")
        .unwrap();
    // Each person row should have the total count
    assert_eq!(result.rows.len(), 3);
}

// ---------------------------------------------------------------------------
// Phase 7: Missing functions
// ---------------------------------------------------------------------------

#[test]
fn test_string_join() {
    let db = setup_db();
    let session = db.session();
    let result = session
        .execute(
            "MATCH (n:Person {name: 'Alice'}) RETURN string_join(['a', 'b', 'c'], '-') AS joined",
        )
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::String("a-b-c".into()));
}

// ---------------------------------------------------------------------------
// Phase 4: SET map operations
// ---------------------------------------------------------------------------

#[test]
fn test_set_map_merge() {
    let db = GrafeoDB::new_in_memory();
    let mut session = db.session();
    session.begin_tx().unwrap();
    session
        .execute("INSERT (:Person {name: 'Dave', age: 40})")
        .unwrap();
    session.commit().unwrap();

    session.begin_tx().unwrap();
    session
        .execute("MATCH (n:Person {name: 'Dave'}) SET n += {city: 'NYC', age: 41}")
        .unwrap();
    session.commit().unwrap();

    let result = session
        .execute("MATCH (n:Person {name: 'Dave'}) RETURN n.age, n.city")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int64(41)); // age updated
    assert_eq!(result.rows[0][1], Value::String("NYC".into())); // city added
}

#[test]
fn test_set_map_replace() {
    let db = GrafeoDB::new_in_memory();
    let mut session = db.session();
    session.begin_tx().unwrap();
    session
        .execute("INSERT (:Person {name: 'Eve', age: 28, city: 'LA'})")
        .unwrap();
    session.commit().unwrap();

    session.begin_tx().unwrap();
    session
        .execute("MATCH (n:Person {name: 'Eve'}) SET n = {name: 'Eve', role: 'admin'}")
        .unwrap();
    session.commit().unwrap();

    let result = session
        .execute("MATCH (n:Person {name: 'Eve'}) RETURN n.age, n.role")
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Null); // age gone (replaced)
    assert_eq!(result.rows[0][1], Value::String("admin".into())); // role set
}
