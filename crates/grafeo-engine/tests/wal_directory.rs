//! Integration tests for WAL directory-format persistence.
//!
//! Proves that data survives a close/reopen cycle when using the legacy
//! WAL directory format, including after WAL rotation.
//!
//! ```bash
//! cargo test -p grafeo-engine --features full --test wal_directory
//! ```

#[cfg(feature = "wal")]
mod wal_directory {
    use grafeo_common::types::Value;
    use grafeo_engine::config::StorageFormat;
    use grafeo_engine::{Config, GrafeoDB};

    /// Basic roundtrip: create nodes and edges, close, reopen, verify.
    #[test]
    fn roundtrip_no_rotation() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("testdb");

        {
            let config = Config::persistent(&path).with_storage_format(StorageFormat::WalDirectory);
            let db = GrafeoDB::with_config(config).expect("open for write");
            let a = db.create_node(&["Person"]);
            db.set_node_property(a, "name", Value::String("Alix".into()));
            let b = db.create_node(&["Person"]);
            db.set_node_property(b, "name", Value::String("Gus".into()));
            db.create_edge(a, b, "KNOWS");
            db.close().expect("close");
        }

        {
            let config = Config::persistent(&path).with_storage_format(StorageFormat::WalDirectory);
            let db = GrafeoDB::with_config(config).expect("reopen");
            assert_eq!(db.node_count(), 2, "expected 2 nodes after reopen");
            assert_eq!(db.edge_count(), 1, "expected 1 edge after reopen");
            db.close().expect("close");
        }
    }

    /// Proves the checkpoint.meta bug: data written before WAL rotation
    /// must survive close/reopen. Without the fix, recovery skips old WAL
    /// files because checkpoint.meta records the current sequence number.
    #[test]
    fn roundtrip_after_wal_rotation() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("rotdb");

        {
            let config = Config::persistent(&path).with_storage_format(StorageFormat::WalDirectory);
            let db = GrafeoDB::with_config(config).expect("open for write");

            // Write nodes BEFORE rotation (these go into wal_0.log)
            let a = db.create_node(&["Person"]);
            db.set_node_property(a, "name", Value::String("Alix".into()));
            let b = db.create_node(&["Person"]);
            db.set_node_property(b, "name", Value::String("Gus".into()));
            db.create_edge(a, b, "KNOWS");

            // Force WAL rotation so current_sequence advances
            let wal = db.wal().expect("WAL should be present");
            wal.rotate().expect("rotate WAL");

            // Write more nodes AFTER rotation (these go into wal_1.log)
            let c = db.create_node(&["Person"]);
            db.set_node_property(c, "name", Value::String("Vincent".into()));

            db.close().expect("close");
        }

        // Reopen and verify ALL data is present
        {
            let config = Config::persistent(&path).with_storage_format(StorageFormat::WalDirectory);
            let db = GrafeoDB::with_config(config).expect("reopen");

            assert_eq!(
                db.node_count(),
                3,
                "expected 3 nodes after reopen (2 before rotation + 1 after)"
            );
            assert_eq!(
                db.edge_count(),
                1,
                "expected 1 edge after reopen (created before rotation)"
            );

            // Verify the node created before rotation is queryable
            let session = db.session();
            let result = session
                .execute("MATCH (n:Person {name: 'Alix'}) RETURN n.name")
                .unwrap();
            assert_eq!(
                result.rows.len(),
                1,
                "node created before WAL rotation should be recoverable"
            );

            db.close().expect("close");
        }
    }

    /// Data accumulates correctly across multiple close/reopen cycles,
    /// including a WAL rotation mid-way through.
    #[test]
    fn multiple_reopen_cycles() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("cycles");
        let open = || {
            let config = Config::persistent(&path).with_storage_format(StorageFormat::WalDirectory);
            GrafeoDB::with_config(config).expect("open")
        };

        // Cycle 1: seed 5 nodes
        {
            let db = open();
            for i in 0..5 {
                db.create_node_with_props(
                    &["Batch"],
                    [("cycle", Value::Int64(1)), ("idx", Value::Int64(i))],
                );
            }
            db.close().expect("close");
        }

        // Cycle 2: verify cycle-1 data, add more, force WAL rotation
        {
            let db = open();
            assert_eq!(db.node_count(), 5);
            for i in 0..10 {
                db.create_node_with_props(
                    &["Batch"],
                    [("cycle", Value::Int64(2)), ("idx", Value::Int64(i))],
                );
            }
            db.wal().expect("wal").rotate().expect("rotate");
            for i in 0..3 {
                db.create_node_with_props(
                    &["Batch"],
                    [("cycle", Value::Int64(2)), ("idx", Value::Int64(10 + i))],
                );
            }
            db.close().expect("close");
        }

        // Cycle 3: all 18 nodes from both cycles must be present
        {
            let db = open();
            assert_eq!(db.node_count(), 18, "5 + 13 nodes across two cycles");

            let session = db.session();
            let result = session
                .execute("MATCH (n:Batch) WHERE n.cycle = 1 RETURN count(n)")
                .unwrap();
            assert_eq!(result.rows[0][0], Value::Int64(5), "cycle-1 nodes intact");

            let result = session
                .execute("MATCH (n:Batch) WHERE n.cycle = 2 RETURN count(n)")
                .unwrap();
            assert_eq!(result.rows[0][0], Value::Int64(13), "cycle-2 nodes intact");
            db.close().expect("close");
        }
    }

    /// Dropping the database without calling `close()` must still persist data.
    /// The `Drop` impl calls `close()` internally.
    #[test]
    fn drop_without_explicit_close() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("dropdb");

        {
            let config = Config::persistent(&path).with_storage_format(StorageFormat::WalDirectory);
            let db = GrafeoDB::with_config(config).expect("open");
            let n = db.create_node(&["Person"]);
            db.set_node_property(n, "name", Value::String("Alix".into()));
            // intentionally no db.close(), Drop handles it
        }

        {
            let config = Config::persistent(&path).with_storage_format(StorageFormat::WalDirectory);
            let db = GrafeoDB::with_config(config).expect("reopen");
            assert_eq!(db.node_count(), 1, "node should survive implicit Drop");

            let session = db.session();
            let result = session.execute("MATCH (n:Person) RETURN n.name").unwrap();
            assert_eq!(
                result.rows[0][0],
                Value::String("Alix".into()),
                "property should survive implicit Drop"
            );
        }
    }

    /// Verify that no checkpoint.meta file is written for directory format.
    /// Its presence would cause recovery to skip WAL files.
    #[test]
    fn no_checkpoint_meta_written() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("nockpt");

        {
            let config = Config::persistent(&path).with_storage_format(StorageFormat::WalDirectory);
            let db = GrafeoDB::with_config(config).expect("open");
            db.create_node(&["Test"]);
            db.close().expect("close");
        }

        let wal_dir = path.join("wal");
        let checkpoint_meta = wal_dir.join("checkpoint.meta");
        assert!(
            !checkpoint_meta.exists(),
            "checkpoint.meta should NOT exist for directory-format WAL"
        );
    }
}
