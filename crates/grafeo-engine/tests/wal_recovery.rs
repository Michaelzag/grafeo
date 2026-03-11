//! Integration tests for WAL-based persistence and recovery.
//!
//! Covers: save/restore cycle, WAL recovery of nodes/edges/properties/labels,
//! and checkpoint operations.
//!
//! ```bash
//! cargo test -p grafeo-engine --features full --test wal_recovery
//! ```

#[cfg(feature = "wal")]
mod wal {
    use grafeo_common::types::Value;
    use grafeo_engine::GrafeoDB;

    #[test]
    fn test_persistent_db_roundtrip() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("test.grafeo");

        // Create and populate
        {
            let db = GrafeoDB::open(&path).expect("open for write");
            let a = db.create_node(&["Person"]);
            db.set_node_property(a, "name", Value::String("Alix".into()));
            db.set_node_property(a, "age", Value::Int64(30));
            let b = db.create_node(&["Person"]);
            db.set_node_property(b, "name", Value::String("Gus".into()));
            db.create_edge(a, b, "KNOWS");
            db.close().expect("close");
        }

        // Reopen and verify
        {
            let db = GrafeoDB::open(&path).expect("reopen");
            assert_eq!(db.node_count(), 2);
            assert_eq!(db.edge_count(), 1);

            let session = db.session();
            let result = session
                .execute("MATCH (n:Person {name: 'Alix'}) RETURN n.age")
                .unwrap();
            assert_eq!(result.rows.len(), 1);
            assert_eq!(result.rows[0][0], Value::Int64(30));

            db.close().expect("close");
        }
    }

    #[test]
    fn test_wal_recovery_labels() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("labels.grafeo");

        {
            let db = GrafeoDB::open(&path).expect("open");
            let n = db.create_node(&["Person"]);
            db.add_node_label(n, "Employee");
            db.close().expect("close");
        }

        {
            let db = GrafeoDB::open(&path).expect("reopen");
            let session = db.session();
            let result = session.execute("MATCH (n:Employee) RETURN n").unwrap();
            assert_eq!(result.rows.len(), 1, "label should survive WAL recovery");
            db.close().expect("close");
        }
    }

    #[test]
    fn test_wal_recovery_deletes() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("deletes.grafeo");

        {
            let db = GrafeoDB::open(&path).expect("open");
            let a = db.create_node(&["Person"]);
            db.set_node_property(a, "name", Value::String("Alix".into()));
            let b = db.create_node(&["Person"]);
            db.set_node_property(b, "name", Value::String("Gus".into()));
            let c = db.create_node(&["Person"]);
            db.set_node_property(c, "name", Value::String("Harm".into()));
            db.create_edge(a, b, "KNOWS");

            // Delete Harm (no edges)
            db.delete_node(c);
            db.close().expect("close");
        }

        {
            let db = GrafeoDB::open(&path).expect("reopen");
            assert_eq!(
                db.node_count(),
                2,
                "deleted node should not survive recovery"
            );
            assert_eq!(db.edge_count(), 1);
            db.close().expect("close");
        }
    }

    #[test]
    fn test_save_to_new_location() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let save_path = dir.path().join("saved.grafeo");

        let db = GrafeoDB::new_in_memory();
        let a = db.create_node(&["Person"]);
        db.set_node_property(a, "name", Value::String("Alix".into()));
        let b = db.create_node(&["Person"]);
        db.set_node_property(b, "name", Value::String("Gus".into()));
        db.create_edge(a, b, "KNOWS");

        // Save in-memory database to disk
        db.save(&save_path).expect("save should succeed");

        // Open the saved database
        let restored = GrafeoDB::open(&save_path).expect("open saved");
        assert_eq!(restored.node_count(), 2);
        assert_eq!(restored.edge_count(), 1);
        restored.close().expect("close");
    }

    #[test]
    fn test_open_in_memory_from_file() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("source.grafeo");

        // Create persistent database
        {
            let db = GrafeoDB::open(&path).expect("open");
            let n = db.create_node(&["Person"]);
            db.set_node_property(n, "name", Value::String("Alix".into()));
            db.close().expect("close");
        }

        // Load into memory (detached from file)
        let mem_db = GrafeoDB::open_in_memory(&path).expect("open_in_memory");
        assert_eq!(mem_db.node_count(), 1);
        assert!(
            !mem_db.is_persistent(),
            "should be in-memory after open_in_memory"
        );

        // Modifications to the in-memory copy don't affect the file
        mem_db.create_node(&["Extra"]);
        assert_eq!(mem_db.node_count(), 2);

        // Reopening the file should still show 1 node
        let file_db = GrafeoDB::open(&path).expect("reopen");
        assert_eq!(file_db.node_count(), 1);
        file_db.close().expect("close");
    }

    #[test]
    fn test_wal_status_persistent() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("status.grafeo");

        let db = GrafeoDB::open(&path).expect("open");
        db.create_node(&["N"]);

        let status = db.wal_status();
        assert!(status.enabled, "persistent db should have WAL enabled");

        db.close().expect("close");
    }

    #[test]
    fn test_query_mutations_persist() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("query_persist.grafeo");

        // Create data via GQL queries (not CRUD API)
        {
            let db = GrafeoDB::open(&path).expect("open");
            let session = db.session();
            session
                .execute("INSERT (:Person {name: 'Alix', age: 30})")
                .expect("insert Alix");
            session
                .execute("INSERT (:Person {name: 'Gus', age: 25})")
                .expect("insert Gus");
            session
                .execute(
                    "MATCH (a:Person {name: 'Alix'}), (b:Person {name: 'Gus'}) \
                     INSERT (a)-[:KNOWS {since: 2020}]->(b)",
                )
                .expect("insert edge");

            // Verify data exists before close
            let result = session
                .execute("MATCH (n:Person) RETURN n.name ORDER BY n.name")
                .unwrap();
            assert_eq!(result.rows.len(), 2);

            db.close().expect("close");
        }

        // Reopen and verify all query-created data survived
        {
            let db = GrafeoDB::open(&path).expect("reopen");
            assert_eq!(db.node_count(), 2, "query-created nodes should persist");
            assert_eq!(db.edge_count(), 1, "query-created edges should persist");

            let session = db.session();
            let result = session
                .execute("MATCH (n:Person {name: 'Alix'}) RETURN n.age")
                .unwrap();
            assert_eq!(result.rows.len(), 1, "Alix should be queryable");
            assert_eq!(result.rows[0][0], Value::Int64(30));

            let result = session
                .execute("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name")
                .unwrap();
            assert_eq!(result.rows.len(), 1, "KNOWS edge should persist");

            db.close().expect("close");
        }
    }

    #[test]
    fn test_query_delete_persists() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("query_delete.grafeo");

        {
            let db = GrafeoDB::open(&path).expect("open");
            let session = db.session();
            session
                .execute("INSERT (:Person {name: 'Alix'})")
                .expect("insert");
            session
                .execute("INSERT (:Person {name: 'Gus'})")
                .expect("insert");
            session
                .execute("MATCH (n:Person {name: 'Gus'}) DELETE n")
                .expect("delete");
            db.close().expect("close");
        }

        {
            let db = GrafeoDB::open(&path).expect("reopen");
            assert_eq!(db.node_count(), 1, "delete should persist");
            let session = db.session();
            let result = session.execute("MATCH (n:Person) RETURN n.name").unwrap();
            assert_eq!(result.rows.len(), 1);
            assert_eq!(result.rows[0][0], Value::String("Alix".into()));
            db.close().expect("close");
        }
    }

    #[test]
    fn test_query_set_property_persists() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("query_set.grafeo");

        {
            let db = GrafeoDB::open(&path).expect("open");
            let session = db.session();
            session
                .execute("INSERT (:Person {name: 'Alix', age: 30})")
                .expect("insert");
            session
                .execute("MATCH (n:Person {name: 'Alix'}) SET n.age = 31")
                .expect("update");
            db.close().expect("close");
        }

        {
            let db = GrafeoDB::open(&path).expect("reopen");
            let session = db.session();
            let result = session
                .execute("MATCH (n:Person {name: 'Alix'}) RETURN n.age")
                .unwrap();
            assert_eq!(result.rows.len(), 1);
            assert_eq!(result.rows[0][0], Value::Int64(31), "SET should persist");
            db.close().expect("close");
        }
    }

    #[test]
    fn test_wal_checkpoint_succeeds() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("checkpoint.grafeo");

        let db = GrafeoDB::open(&path).expect("open");
        let n = db.create_node(&["Person"]);
        db.set_node_property(n, "name", Value::String("Alix".into()));

        // Explicit checkpoint should not error
        db.wal_checkpoint().expect("checkpoint should succeed");

        // Verify data is still accessible after checkpoint
        assert_eq!(db.node_count(), 1);

        db.close().expect("close");
    }

    // =========================================================================
    // Named Graph WAL Recovery
    // =========================================================================

    #[test]
    fn test_named_graph_persists_across_restart() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("named_graph.grafeo");

        {
            let db = GrafeoDB::open(&path).expect("open");
            let session = db.session();
            session.execute("CREATE GRAPH analytics").unwrap();
            session.execute("USE GRAPH analytics").unwrap();
            session
                .execute("INSERT (:KPI {name: 'pageviews', count: 42})")
                .unwrap();
            assert_eq!(db.node_count(), 0, "default graph should be empty");
            db.close().expect("close");
        }

        {
            let db = GrafeoDB::open(&path).expect("reopen");
            let session = db.session();
            // Default graph should still be empty
            assert_eq!(db.node_count(), 0);

            // Named graph should have the data
            session.execute("USE GRAPH analytics").unwrap();
            let result = session
                .execute("MATCH (m:KPI) RETURN m.name, m.count")
                .unwrap();
            assert_eq!(result.rows.len(), 1);
            assert_eq!(result.rows[0][0], Value::String("pageviews".into()));
            assert_eq!(result.rows[0][1], Value::Int64(42));
            db.close().expect("close");
        }
    }

    #[test]
    fn test_drop_named_graph_persists() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("drop_graph.grafeo");

        {
            let db = GrafeoDB::open(&path).expect("open");
            let session = db.session();
            session.execute("CREATE GRAPH temp_graph").unwrap();
            session.execute("USE GRAPH temp_graph").unwrap();
            session.execute("INSERT (:Temp {val: 1})").unwrap();
            // Drop the graph we just created
            session.execute("USE GRAPH default").unwrap();
            session.execute("DROP GRAPH temp_graph").unwrap();
            db.close().expect("close");
        }

        {
            let db = GrafeoDB::open(&path).expect("reopen");
            let session = db.session();
            // Trying to use the dropped graph should fail
            let result = session.execute("USE GRAPH temp_graph");
            assert!(
                result.is_err(),
                "dropped graph should not exist after recovery"
            );
            db.close().expect("close");
        }
    }

    #[test]
    fn test_multiple_named_graphs_persist() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("multi_graph.grafeo");

        {
            let db = GrafeoDB::open(&path).expect("open");
            let session = db.session();

            // Create data in default graph
            session.execute("INSERT (:Person {name: 'Alix'})").unwrap();

            // Create data in graph "alpha"
            session.execute("CREATE GRAPH alpha").unwrap();
            session.execute("USE GRAPH alpha").unwrap();
            session.execute("INSERT (:Item {name: 'Widget'})").unwrap();

            // Create data in graph "beta"
            session.execute("USE GRAPH default").unwrap();
            session.execute("CREATE GRAPH beta").unwrap();
            session.execute("USE GRAPH beta").unwrap();
            session
                .execute("INSERT (:City {name: 'Amsterdam'})")
                .unwrap();

            db.close().expect("close");
        }

        {
            let db = GrafeoDB::open(&path).expect("reopen");
            let session = db.session();

            // Check default graph
            let result = session.execute("MATCH (p:Person) RETURN p.name").unwrap();
            assert_eq!(result.rows.len(), 1);
            assert_eq!(result.rows[0][0], Value::String("Alix".into()));

            // Check alpha graph
            session.execute("USE GRAPH alpha").unwrap();
            let result = session.execute("MATCH (i:Item) RETURN i.name").unwrap();
            assert_eq!(result.rows.len(), 1);
            assert_eq!(result.rows[0][0], Value::String("Widget".into()));

            // Check beta graph
            session.execute("USE GRAPH beta").unwrap();
            let result = session.execute("MATCH (c:City) RETURN c.name").unwrap();
            assert_eq!(result.rows.len(), 1);
            assert_eq!(result.rows[0][0], Value::String("Amsterdam".into()));

            db.close().expect("close");
        }
    }

    #[test]
    fn test_named_graph_wal_interleaved_with_default() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("interleaved.grafeo");

        {
            let db = GrafeoDB::open(&path).expect("open");
            let session = db.session();

            // Interleave mutations between default and named graph
            session.execute("INSERT (:Person {name: 'Alix'})").unwrap();
            session.execute("CREATE GRAPH other").unwrap();
            session.execute("USE GRAPH other").unwrap();
            session.execute("INSERT (:Robot {name: 'R2D2'})").unwrap();
            session.execute("USE GRAPH default").unwrap();
            session.execute("INSERT (:Person {name: 'Gus'})").unwrap();
            session.execute("USE GRAPH other").unwrap();
            session.execute("INSERT (:Robot {name: 'C3PO'})").unwrap();

            db.close().expect("close");
        }

        {
            let db = GrafeoDB::open(&path).expect("reopen");
            let session = db.session();

            // Default graph: 2 persons
            let result = session
                .execute("MATCH (p:Person) RETURN p.name ORDER BY p.name")
                .unwrap();
            assert_eq!(result.rows.len(), 2);
            assert_eq!(result.rows[0][0], Value::String("Alix".into()));
            assert_eq!(result.rows[1][0], Value::String("Gus".into()));

            // Other graph: 2 robots
            session.execute("USE GRAPH other").unwrap();
            let result = session
                .execute("MATCH (r:Robot) RETURN r.name ORDER BY r.name")
                .unwrap();
            assert_eq!(result.rows.len(), 2);
            assert_eq!(result.rows[0][0], Value::String("C3PO".into()));
            assert_eq!(result.rows[1][0], Value::String("R2D2".into()));

            db.close().expect("close");
        }
    }

    #[test]
    fn test_save_preserves_named_graphs() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let save_path = dir.path().join("saved_graphs.grafeo");

        let db = GrafeoDB::new_in_memory();
        let session = db.session();

        session.execute("INSERT (:Person {name: 'Alix'})").unwrap();
        session.execute("CREATE GRAPH analytics").unwrap();
        session.execute("USE GRAPH analytics").unwrap();
        session
            .execute("INSERT (:KPI {name: 'views', count: 100})")
            .unwrap();

        db.save(&save_path).expect("save should succeed");

        let restored = GrafeoDB::open(&save_path).expect("open saved");
        assert_eq!(restored.node_count(), 1, "default graph: 1 person");

        let session2 = restored.session();
        session2.execute("USE GRAPH analytics").unwrap();
        let result = session2.execute("MATCH (m:KPI) RETURN m.name").unwrap();
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], Value::String("views".into()));
        restored.close().expect("close");
    }
}
