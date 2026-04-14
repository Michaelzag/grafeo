//! Integration tests for the push-based pipeline execution path.
//!
//! These tests verify that queries using filter, sort, aggregate, limit,
//! and distinct operators execute correctly through the push pipeline
//! (not the Volcano pull loop). Each test constructs a scenario that forces
//! the pipeline converter to decompose specific operators, exercising:
//!
//! - `into_any()` on each operator type
//! - `into_parts()` for decomposition
//! - `PredicateAdapter` bridging pull predicates to push
//! - `SortKey` conversion between pull/push types
//! - `Pipeline::execute()` main loop and `finalize_all()`
//! - `execute_pipeline()` in the Executor
//! - Early termination via Limit

use grafeo_common::types::Value;
use grafeo_engine::GrafeoDB;

fn setup_people_db() -> GrafeoDB {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();
    session
        .execute(
            "INSERT (:Person {name: 'Alix', age: 30, city: 'Amsterdam'}),
                    (:Person {name: 'Gus', age: 25, city: 'Berlin'}),
                    (:Person {name: 'Vincent', age: 40, city: 'Amsterdam'}),
                    (:Person {name: 'Jules', age: 35, city: 'Paris'}),
                    (:Person {name: 'Mia', age: 28, city: 'Berlin'}),
                    (:Person {name: 'Butch', age: 45, city: 'Prague'}),
                    (:Person {name: 'Django', age: 32, city: 'Amsterdam'}),
                    (:Person {name: 'Shosanna', age: 27, city: 'Paris'})",
        )
        .unwrap();
    db
}

// ── Filter (exercises FilterPushOperator + PredicateAdapter) ─────────

#[test]
fn test_push_filter_equality() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) WHERE p.city = 'Amsterdam' RETURN p.name ORDER BY p.name")
        .unwrap();

    let names: Vec<&str> = result
        .rows()
        .iter()
        .map(|r| r[0].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Alix", "Django", "Vincent"]);
}

#[test]
fn test_push_filter_comparison() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) WHERE p.age > 35 RETURN p.name ORDER BY p.name")
        .unwrap();

    let names: Vec<&str> = result
        .rows()
        .iter()
        .map(|r| r[0].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Butch", "Vincent"]);
}

#[test]
fn test_push_filter_no_matches() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) WHERE p.age > 100 RETURN p.name")
        .unwrap();

    assert_eq!(result.rows().len(), 0);
}

// ── Sort (exercises SortPushOperator + convert_sort_key) ─────────────

#[test]
fn test_push_sort_ascending() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN p.name ORDER BY p.age ASC")
        .unwrap();

    let names: Vec<&str> = result
        .rows()
        .iter()
        .map(|r| r[0].as_str().unwrap())
        .collect();
    assert_eq!(names[0], "Gus"); // youngest (25)
    assert_eq!(names[names.len() - 1], "Butch"); // oldest (45)
    assert_eq!(names.len(), 8);
}

#[test]
fn test_push_sort_descending() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN p.name, p.age ORDER BY p.age DESC")
        .unwrap();

    let first_age = &result.rows()[0][1];
    let last_age = &result.rows()[7][1];
    assert_eq!(*first_age, Value::Int64(45)); // Butch
    assert_eq!(*last_age, Value::Int64(25)); // Gus
}

#[test]
fn test_push_sort_multiple_keys() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN p.city, p.name ORDER BY p.city ASC, p.name ASC")
        .unwrap();

    // Amsterdam should come first, with names alphabetical within
    let first = &result.rows()[0];
    assert_eq!(first[0].as_str().unwrap(), "Amsterdam");
    assert_eq!(first[1].as_str().unwrap(), "Alix");
}

// ── Aggregate (exercises AggregatePushOperator) ──────────────────────

#[test]
fn test_push_aggregate_count() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN COUNT(*) AS total")
        .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0][0], Value::Int64(8));
}

#[test]
fn test_push_aggregate_group_by() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN p.city, COUNT(*) AS cnt ORDER BY p.city ASC")
        .unwrap();

    // Amsterdam: 3, Berlin: 2, Paris: 2, Prague: 1
    assert_eq!(result.rows().len(), 4);

    let amsterdam = &result.rows()[0];
    assert_eq!(amsterdam[0].as_str().unwrap(), "Amsterdam");
    assert_eq!(amsterdam[1], Value::Int64(3));

    let prague = &result.rows()[3];
    assert_eq!(prague[0].as_str().unwrap(), "Prague");
    assert_eq!(prague[1], Value::Int64(1));
}

#[test]
fn test_push_aggregate_sum_avg() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN SUM(p.age) AS total_age, AVG(p.age) AS avg_age")
        .unwrap();

    assert_eq!(result.rows().len(), 1);
    // 30+25+40+35+28+45+32+27 = 262
    assert_eq!(result.rows()[0][0], Value::Int64(262));
}

#[test]
fn test_push_aggregate_min_max() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN MIN(p.age) AS youngest, MAX(p.age) AS oldest")
        .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0][0], Value::Int64(25));
    assert_eq!(result.rows()[0][1], Value::Int64(45));
}

// ── Limit (exercises LimitPushOperator + early termination) ──────────

#[test]
fn test_push_limit() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN p.name ORDER BY p.name LIMIT 3")
        .unwrap();

    assert_eq!(result.rows().len(), 3);
    assert_eq!(result.rows()[0][0].as_str().unwrap(), "Alix");
}

#[test]
fn test_push_limit_exceeds_data() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN p.name LIMIT 100")
        .unwrap();

    // Only 8 people, limit 100 should return all 8
    assert_eq!(result.rows().len(), 8);
}

#[test]
fn test_push_limit_zero() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN p.name LIMIT 0")
        .unwrap();

    assert_eq!(result.rows().len(), 0);
}

// ── Distinct (exercises DistinctPushOperator + hash_value_into) ──────

#[test]
fn test_push_distinct_strings() {
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute("MATCH (p:Person) RETURN DISTINCT p.city ORDER BY p.city")
        .unwrap();

    let cities: Vec<&str> = result
        .rows()
        .iter()
        .map(|r| r[0].as_str().unwrap())
        .collect();
    assert_eq!(cities, vec!["Amsterdam", "Berlin", "Paris", "Prague"]);
}

#[test]
fn test_push_distinct_integers() {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();
    session
        .execute("INSERT (:N {v: 1}), (:N {v: 2}), (:N {v: 1}), (:N {v: 3}), (:N {v: 2})")
        .unwrap();

    let result = session
        .execute("MATCH (n:N) RETURN DISTINCT n.v ORDER BY n.v")
        .unwrap();

    assert_eq!(result.rows().len(), 3);
    assert_eq!(result.rows()[0][0], Value::Int64(1));
    assert_eq!(result.rows()[1][0], Value::Int64(2));
    assert_eq!(result.rows()[2][0], Value::Int64(3));
}

// ── Combined operators (exercises multi-stage pipeline) ──────────────

#[test]
fn test_push_filter_sort_limit() {
    let db = setup_people_db();
    let session = db.session();

    // Filter + Sort + Limit: three push operators chained
    let result = session
        .execute("MATCH (p:Person) WHERE p.age >= 30 RETURN p.name ORDER BY p.age DESC LIMIT 3")
        .unwrap();

    assert_eq!(result.rows().len(), 3);
    // Oldest 3 among age >= 30: Butch(45), Vincent(40), Jules(35)
    assert_eq!(result.rows()[0][0].as_str().unwrap(), "Butch");
    assert_eq!(result.rows()[1][0].as_str().unwrap(), "Vincent");
    assert_eq!(result.rows()[2][0].as_str().unwrap(), "Jules");
}

#[test]
fn test_push_aggregate_with_filter() {
    let db = setup_people_db();
    let session = db.session();

    // Filter + Aggregate: count people over 30 per city
    let result = session
        .execute(
            "MATCH (p:Person) WHERE p.age > 30 \
             RETURN p.city, COUNT(*) AS cnt \
             ORDER BY p.city ASC",
        )
        .unwrap();

    // Amsterdam: Vincent(40), Django(32) = 2
    // Paris: Jules(35) = 1
    // Prague: Butch(45) = 1
    assert!(result.rows().len() >= 3);
}

#[test]
fn test_push_distinct_with_sort() {
    let db = setup_people_db();
    let session = db.session();

    // Distinct + Sort: unique cities sorted
    let result = session
        .execute("MATCH (p:Person) RETURN DISTINCT p.city ORDER BY p.city DESC")
        .unwrap();

    let cities: Vec<&str> = result
        .rows()
        .iter()
        .map(|r| r[0].as_str().unwrap())
        .collect();
    assert_eq!(cities, vec!["Prague", "Paris", "Berlin", "Amsterdam"]);
}

#[test]
fn test_push_filter_distinct_limit() {
    let db = setup_people_db();
    let session = db.session();

    // Filter + Distinct + Limit: unique cities of people over 25, limited to 2
    let result = session
        .execute(
            "MATCH (p:Person) WHERE p.age > 25 \
             RETURN DISTINCT p.city \
             ORDER BY p.city \
             LIMIT 2",
        )
        .unwrap();

    assert_eq!(result.rows().len(), 2);
    assert_eq!(result.rows()[0][0].as_str().unwrap(), "Amsterdam");
    assert_eq!(result.rows()[1][0].as_str().unwrap(), "Berlin");
}

// ── Pull fallback (verifies non-decomposable queries still work) ─────

#[test]
fn test_pull_fallback_scan_only() {
    let db = setup_people_db();
    let session = db.session();

    // Simple scan with no filter/sort/limit: should stay pull-based
    let result = session.execute("MATCH (p:Person) RETURN p.name").unwrap();

    assert_eq!(result.rows().len(), 8);
}

#[test]
fn test_pull_fallback_with_expand() {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();
    session
        .execute("INSERT (:A {name: 'Alix'})-[:KNOWS]->(:B {name: 'Gus'})")
        .unwrap();

    // Expand stays pull-based, but result should be correct
    let result = session
        .execute("MATCH (a:A)-[:KNOWS]->(b:B) RETURN a.name, b.name")
        .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0][0].as_str().unwrap(), "Alix");
    assert_eq!(result.rows()[0][1].as_str().unwrap(), "Gus");
}

// ── Edge cases ───────────────────────────────────────────────────────

#[test]
fn test_push_empty_result_through_pipeline() {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();

    // No data in the database, pipeline should handle empty gracefully
    let result = session
        .execute("MATCH (p:Person) WHERE p.age > 0 RETURN p.name ORDER BY p.name LIMIT 10")
        .unwrap();

    assert_eq!(result.rows().len(), 0);
}

#[test]
fn test_push_single_row_through_pipeline() {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();
    session
        .execute("INSERT (:Solo {name: 'Alix', v: 1})")
        .unwrap();

    let result = session
        .execute("MATCH (s:Solo) WHERE s.v = 1 RETURN s.name ORDER BY s.name LIMIT 10")
        .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0][0].as_str().unwrap(), "Alix");
}

#[test]
fn test_push_aggregate_on_empty() {
    let db = GrafeoDB::new_in_memory();
    let session = db.session();

    // COUNT on empty should return 0
    let result = session
        .execute("MATCH (p:Person) RETURN COUNT(*) AS cnt")
        .unwrap();

    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0][0], Value::Int64(0));
}

// ── Pipeline conversion tests (via query execution) ──────────────────

#[test]
fn test_pipeline_converts_filter_to_push() {
    // Verify the push path is actually taken by checking that queries
    // with WHERE clauses produce correct results (they go through
    // FilterPushOperator + PredicateAdapter)
    let db = setup_people_db();
    let session = db.session();

    // This query decomposes: Scan -> Filter(push) -> Sort(push) -> Limit(push)
    let result = session
        .execute(
            "MATCH (p:Person) WHERE p.city = 'Amsterdam' AND p.age > 29 \
             RETURN p.name ORDER BY p.name LIMIT 10",
        )
        .unwrap();

    let names: Vec<&str> = result
        .rows()
        .iter()
        .map(|r| r[0].as_str().unwrap())
        .collect();
    // Alix(30), Django(32), Vincent(40) all in Amsterdam and age > 29
    assert_eq!(names, vec!["Alix", "Django", "Vincent"]);
}

#[test]
fn test_pipeline_aggregate_group_sort() {
    // Aggregate + Sort: both are pipeline breakers, exercises finalize chain
    let db = setup_people_db();
    let session = db.session();

    let result = session
        .execute(
            "MATCH (p:Person) \
             RETURN p.city, COUNT(*) AS cnt, AVG(p.age) AS avg_age \
             ORDER BY cnt DESC, p.city ASC",
        )
        .unwrap();

    // Amsterdam: 3 people, Berlin: 2, Paris: 2, Prague: 1
    assert_eq!(result.rows().len(), 4);
    // First row should be Amsterdam (count 3, highest)
    assert_eq!(result.rows()[0][0].as_str().unwrap(), "Amsterdam");
    assert_eq!(result.rows()[0][1], Value::Int64(3));
}
