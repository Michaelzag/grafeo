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
