---
title: Compact Store
description: Convert a database to a read-only columnar format for faster queries and lower memory usage.
tags:
  - performance
  - storage
  - compact-store
  - wasm
---

# Compact Store

CompactStore is a read-only columnar graph format that trades mutability for performance.
After ingesting data (read-write), call `compact()` to switch the database to a columnar
layout with CSR adjacency. Queries keep working across all supported languages, but writes
are rejected.

**When to use it:** workloads that ingest once, then query many times. Code analysis tools,
static knowledge graphs, pre-built datasets for WASM or edge deployments.

## Performance

Measured on the same data, CompactStore vs the standard mutable LpgStore:

| Metric | LpgStore | CompactStore | Improvement |
|--------|----------|--------------|-------------|
| Memory per node (degree 5) | ~3,200 bytes | ~51 bytes | **63x** |
| Edge traversal (10K lookups) | 619 us | 5.3 us | **116x** |
| Property random access (10K) | 123 us | 10 us | **12x** |

The gains come from eliminating MVCC version chains, read locks, hash lookups, and
chunk decompression. CompactStore replaces those with array indexing and contiguous
memory reads.

## Quick Start

=== "Python"

    ```python
    import grafeo

    db = grafeo.GrafeoDB()

    # Ingest data (read-write phase)
    db.execute("INSERT (:Person {name: 'Alix', age: 30})")
    db.execute("INSERT (:Person {name: 'Gus', age: 25})")
    db.execute("INSERT (:City {name: 'Amsterdam'})")
    db.execute("""
        MATCH (p:Person {name: 'Alix'}), (c:City {name: 'Amsterdam'})
        INSERT (p)-[:LIVES_IN]->(c)
    """)

    # Switch to compact mode (read-only from here)
    db.compact()

    # Queries work as before, but faster
    result = db.execute("MATCH (p:Person)-[:LIVES_IN]->(c:City) RETURN p.name, c.name")
    ```

=== "Node.js"

    ```typescript
    import { GrafeoDB } from '@grafeo-db/node';

    const db = GrafeoDB.create();

    await db.execute("INSERT (:Person {name: 'Alix', age: 30})");
    await db.execute("INSERT (:City {name: 'Amsterdam'})");
    await db.execute(`
        MATCH (p:Person {name: 'Alix'}), (c:City {name: 'Amsterdam'})
        INSERT (p)-[:LIVES_IN]->(c)
    `);

    db.compact();

    const result = await db.execute(
        "MATCH (p:Person)-[:LIVES_IN]->(c:City) RETURN p.name, c.name"
    );
    ```

=== "WASM"

    ```javascript
    import init, { Database } from '@grafeo-db/wasm';
    await init();

    const db = new Database();
    db.execute("INSERT (:Person {name: 'Alix', age: 30})");
    db.execute("INSERT (:City {name: 'Amsterdam'})");

    db.compact();

    const result = db.execute(
        "MATCH (p:Person)-[:LIVES_IN]->(c:City) RETURN p.name, c.name"
    );
    ```

=== "C"

    ```c
    #include "grafeo.h"

    GrafeoDatabase *db = grafeo_open_memory();

    grafeo_execute(db, "INSERT (:Person {name: 'Alix', age: 30})");
    grafeo_execute(db, "INSERT (:City {name: 'Amsterdam'})");

    grafeo_compact(db);

    GrafeoResult *r = grafeo_execute(db,
        "MATCH (p:Person) RETURN p.name");
    ```

=== "Rust"

    ```rust
    use grafeo::GrafeoDB;

    let mut db = GrafeoDB::new_in_memory();

    db.execute("INSERT (:Person {name: 'Alix', age: 30})")?;
    db.execute("INSERT (:City {name: 'Amsterdam'})")?;

    db.compact()?;

    let result = db.execute(
        "MATCH (p:Person)-[:LIVES_IN]->(c:City) RETURN p.name, c.name"
    )?;
    ```

## How It Works

`compact()` performs four steps:

1. **Scans** all nodes from the current store, grouped by label
2. **Infers** column types from property values and builds per-label columnar tables
3. **Builds** forward and backward CSR adjacency for each edge type
4. **Swaps** the database to read-only mode and drops the original store

The result is a `CompactStore` backed by:

- **Per-label columnar tables** with typed codecs (bit-packed integers, dictionary-encoded
  strings, boolean bitmaps)
- **Double-indexed CSR** (Compressed Sparse Row) for O(degree) forward and backward traversal
- **Zone maps** (min/max statistics per column) for predicate pushdown

## Type Mapping

Property values are automatically mapped to the most efficient columnar codec:

| Value type | Codec | Notes |
|------------|-------|-------|
| `Int64` (non-negative) | BitPacked | Auto-determined bit width |
| `Bool` | Bitmap | 1 bit per value |
| `String` | Dictionary | Deduplicated string table |
| `Float64` | Dictionary | Serialized as string |
| Negative `Int64` | Dictionary | Serialized as string |
| `List`, `Map`, `Timestamp`, etc. | Dictionary | Serialized as string |

!!! note
    Dictionary-encoded fallback preserves data but loses typed semantics for comparison
    and range queries. If your workload depends on numeric range scans over `Float64` or
    negative integers, keep the standard LpgStore.

## Limitations

- **Read-only**: all write queries (`INSERT`, `CREATE`, `SET`, `DELETE`) fail after `compact()`
- **No undo**: you cannot switch back to read-write mode
- **Multi-label nodes**: nodes with multiple labels are stored under a compound key
  (e.g., `"Actor|Person"`, sorted alphabetically). A query like `MATCH (n:Person)` will
  not match nodes stored under `"Actor|Person"`. Use a single label per node for best results.
- **No disk serialization**: `compact()` operates in memory. To persist a compacted database,
  use snapshot export (WASM) or save before compacting.

## Feature Flag

CompactStore requires the `compact-store` feature flag, which is included by default in:

| Profile | Includes `compact-store` |
|---------|--------------------------|
| `embedded` | Yes (Python, Node.js, C) |
| `browser` | Yes (WASM) |
| `server` | Yes (via `full`) |

For custom builds: `cargo build --features compact-store`.
