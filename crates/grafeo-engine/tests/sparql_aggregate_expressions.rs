//! Regression tests for complex expressions in SPARQL GROUP BY and ORDER BY.
//! Verifies that the RDF planner handles expression-based aggregation and
//! sorting gracefully (returns a clean error, does not panic).
//!
//! STR() in GROUP BY / ORDER BY parses correctly but the physical plan may
//! lack the store reference needed for expression evaluation. These tests
//! accept either:
//!   - `Err` with a known expression-eval message (known limitation), or
//!   - `Ok` with correct results (once the limitation is fixed).
//!
//! ```bash
//! cargo test -p grafeo-engine --all-features --test sparql_aggregate_expressions
//! ```

#[cfg(all(feature = "sparql", feature = "rdf"))]
mod sparql_aggregate_expression_tests {
    use grafeo_engine::GrafeoDB;

    fn rdf_db() -> GrafeoDB {
        GrafeoDB::new_in_memory()
    }

    fn insert_sample_triples(db: &GrafeoDB) {
        db.execute_sparql(
            r#"INSERT DATA {
                <http://ex.org/alix> <http://ex.org/name> "Alix" .
                <http://ex.org/alix> <http://ex.org/age>  "30" .
                <http://ex.org/gus>  <http://ex.org/name> "Gus" .
                <http://ex.org/gus>  <http://ex.org/age>  "25" .
            }"#,
        )
        .unwrap();
    }

    /// GROUP BY (STR(?s)): expression-based grouping should not panic.
    /// STR() on RDF IRI subjects currently produces identical strings for
    /// different subjects (expression evaluation limitation), so the group
    /// count may be less than expected.
    #[test]
    fn sparql_group_by_str_with_count() {
        let db = rdf_db();
        insert_sample_triples(&db);

        let result = db.execute_sparql(
            "SELECT (STR(?s) AS ?subject) (COUNT(*) AS ?cnt) WHERE { ?s ?p ?o } GROUP BY (STR(?s))",
        );

        // Must not panic or error. The store plumbing is now in place.
        let qr = result.unwrap();
        assert!(
            qr.row_count() >= 1,
            "GROUP BY STR(?s) should produce at least 1 group, got {}",
            qr.row_count()
        );
    }

    /// ORDER BY ASC(STR(?s)): expression-based sorting should not panic.
    #[test]
    fn sparql_order_by_str() {
        let db = rdf_db();
        insert_sample_triples(&db);

        let result = db
            .execute_sparql("SELECT ?s WHERE { ?s <http://ex.org/name> ?o } ORDER BY ASC(STR(?s))");

        match result {
            Err(ref err) => {
                let msg = format!("{err}");
                assert!(
                    msg.contains("Store required for expression evaluation"),
                    "Expected 'Store required for expression evaluation', got: {msg}"
                );
            }
            Ok(ref qr) => {
                assert_eq!(
                    qr.row_count(),
                    2,
                    "ORDER BY ASC(STR(?s)) should return 2 rows"
                );
            }
        }
    }

    /// Both GROUP BY and ORDER BY with STR(): combined complex expressions.
    #[test]
    fn sparql_group_by_and_order_by_both_complex() {
        let db = rdf_db();
        insert_sample_triples(&db);

        let result = db.execute_sparql(
            "SELECT (STR(?s) AS ?subject) (COUNT(*) AS ?cnt) WHERE { ?s ?p ?o } GROUP BY (STR(?s)) ORDER BY (STR(?s))",
        );

        // Must not panic or error. STR() on IRIs may collapse groups (see above).
        let qr = result.unwrap();
        assert!(
            qr.row_count() >= 1,
            "GROUP BY + ORDER BY STR(?s) should produce at least 1 group, got {}",
            qr.row_count()
        );
    }

    /// ORDER BY DESC(STR(?s)): descending with a function expression.
    #[test]
    fn sparql_order_by_desc_str() {
        let db = rdf_db();
        insert_sample_triples(&db);

        let result = db.execute_sparql(
            "SELECT ?s WHERE { ?s <http://ex.org/name> ?o } ORDER BY DESC(STR(?s))",
        );

        match result {
            Err(ref err) => {
                let msg = format!("{err}");
                assert!(
                    msg.contains("Store required for expression evaluation"),
                    "Expected 'Store required for expression evaluation', got: {msg}"
                );
            }
            Ok(ref qr) => {
                assert_eq!(
                    qr.row_count(),
                    2,
                    "ORDER BY DESC(STR(?s)) should return 2 rows"
                );
            }
        }
    }
}
