---
title: Persistent Storage
description: Using Grafeo with durable storage.
tags:
  - persistence
  - storage
---

# Persistent Storage

Persistent mode stores data durably on disk.

## Creating a Persistent Database

=== "Python"

    ```python
    import grafeo

    db = grafeo.GrafeoDB(path="my_graph.db")
    ```

=== "Rust"

    ```rust
    use grafeo::GrafeoDB;

    let db = GrafeoDB::new("my_graph.db")?;
    ```

## File Structure

```
my_graph.db/
├── data/           # Main data files
├── wal/            # Write-ahead log
└── metadata        # Database metadata
```

## Durability Guarantees

- **Write-Ahead Logging (WAL)** - All changes logged before applying
- **Checkpointing** - Periodic consolidation of WAL into data files
- **Crash Recovery** - Automatic recovery from WAL on startup

## Configuration

```python
db = grafeo.GrafeoDB(
    path="my_graph.db",
    # Sync mode: 'full' (default), 'normal', 'off'
    sync_mode='full'
)
```

| Sync Mode | Durability | Performance |
|-----------|------------|-------------|
| `full` | Highest | Slower |
| `normal` | Good | Faster |
| `off` | None | Fastest |

## Single-File Format (`.grafeo`)

Since 0.5.21, Grafeo supports a single-file database format. The entire database is stored in one `.grafeo` file with a sidecar WAL directory for crash safety.

=== "Python"

    ```python
    db = grafeo.GrafeoDB(path="my_graph.grafeo")
    ```

=== "Rust"

    ```rust
    let db = GrafeoDB::new("my_graph.grafeo")?;
    ```

Features:

- Dual-header crash safety with CRC32 checksums
- Automatic format detection: `.grafeo` extension uses single-file mode, directory paths use multi-file mode
- Exclusive file locking prevents multiple processes from opening the same file simultaneously

## Read-Only Mode

Open a database in read-only mode to allow multiple processes to read the same `.grafeo` file concurrently. Mutations are rejected at the session level.

=== "Python"

    ```python
    db = grafeo.GrafeoDB.open_read_only("my_graph.grafeo")
    ```

=== "Rust"

    ```rust
    let db = GrafeoDB::open_read_only("my_graph.grafeo")?;
    ```

Read-only mode uses a shared file lock instead of an exclusive lock, so multiple readers can coexist.

## Reopening a Database

```python
# First session
db = grafeo.GrafeoDB(path="my_graph.db")
db.execute("INSERT (:Person {name: 'Alix'})")

# Later session: data persists
db = grafeo.GrafeoDB(path="my_graph.db")
result = db.execute("MATCH (p:Person) RETURN p.name")
# Returns 'Alix'
```

## Snapshots

Save and restore database snapshots for backup or migration:

=== "Python"

    ```python
    # Export snapshot
    data = db.snapshot()

    # Import snapshot (atomic, with pre-validation)
    db.restore_snapshot(data)

    # Save to file
    db.save("backup.grafeo")
    ```

=== "Rust"

    ```rust
    // Export
    let data = db.snapshot()?;

    // Restore (validates before applying)
    db.restore_snapshot(&data)?;
    ```

Snapshots include all nodes, edges, properties, labels, schema definitions, index metadata and named graph data. The current format is v4, which also preserves temporal version history.
