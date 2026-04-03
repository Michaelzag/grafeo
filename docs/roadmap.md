# Roadmap

Grafeo is a high-performance, embeddable graph database written in Rust. This roadmap shows where the project has been, where it is now and where it's going. Priorities may shift based on community feedback and real-world usage.

For detailed release notes, see the [CHANGELOG](changelog.md).

---

## Completed Releases

### 0.1: Foundation

Established the core graph engine: labeled property graph (LPG) storage with MVCC transactions, WAL persistence and multiple index types (hash, B-tree, trie, adjacency). Shipped the GQL (ISO standard) parser as the primary query language, with experimental support for Cypher, SPARQL, Gremlin and GraphQL. Python bindings via PyO3 from day one.

### 0.2: Performance

Made the engine competitive on query throughput. Factorized query processing eliminates Cartesian products in multi-hop traversals. Worst-case optimal joins (Leapfrog TrieJoin) handle cyclic patterns efficiently. Lock-free concurrent reads, query plan caching and direct lookup APIs brought large speedups on common access patterns. First graph algorithms (community detection, clustering coefficient, BFS shortest path). Added RDF Ring Index, Block-STM parallel transactions, tiered storage and succinct data structures as opt-in features.

### 0.3: AI Compatibility

Added first-class vector support: `Value::Vector` type, HNSW approximate nearest neighbor index, four distance metrics (cosine, euclidean, dot product, manhattan) with SIMD acceleration (AVX2, SSE, NEON). Vector quantization (scalar, binary, product) for memory-constrained deployments. Hybrid graph + vector queries across all supported query languages. Serializable snapshot isolation for write-heavy workloads.

### 0.4: Developer Accessibility

Expanded the binding surface: Node.js/TypeScript (napi-rs), Go (C FFI), WebAssembly (wasm-bindgen, 660 KB gzipped), SQL/PGQ (SQL:2023 GRAPH_TABLE). Shipped grafeo-cli with interactive shell, filtered/MMR vector search with incremental indexing. All public API items documented.

---

## Current: 0.5, Beta

*Preparing for production readiness.*

The beta series focuses on correctness, completeness and real-world durability. Key areas:

**Search and retrieval**: BM25 text search, hybrid search (BM25 + vector via RRF/weighted fusion), optional in-process ONNX embeddings, MMR for diverse RAG results.

**Graph algorithms**: CALL procedure interface exposing all 22 algorithms (PageRank, Dijkstra, Louvain, SSSP, etc.) from GQL, Cypher and SQL/PGQ. Algorithms themselves were introduced in 0.2, the query-callable interface is new in 0.5.

**Data management**: Temporal types (Date, Time, Duration, DateTime), graph type enforcement with schema validation and constraints, LOAD DATA for CSV/JSONL/Parquet, named graph persistence, cross-graph transactions.

**Transaction correctness**: MVCC dirty-read prevention, DELETE rollback with full undo log, write-write conflict detection, session auto-rollback, savepoints.

**Persistence**: Single-file `.grafeo` format with dual-header crash safety, index metadata persistence (snapshot v4), read-only open mode with shared file lock for concurrent reader processes.

**Bindings**: C#/.NET 8, Dart/Flutter (community contribution), C FFI layer shared by Go, and C#.

**Ecosystem**: [grafeo-server](https://github.com/GrafeoDB/grafeo-server), [grafeo-web](https://github.com/GrafeoDB/grafeo-web), [grafeo-mcp](https://github.com/GrafeoDB/grafeo-mcp), [grafeo-memory](https://github.com/GrafeoDB/grafeo-memory), [grafeo-langchain](https://github.com/GrafeoDB/grafeo-langchain), [grafeo-llamaindex](https://github.com/GrafeoDB/grafeo-llamaindex).

### Delivered in 0.5.30-0.5.32

- **Async storage backend** (0.5.30): `AsyncStorageBackend` trait, `AsyncTypedWal` with async WAL operations, `AsyncLocalBackend` filesystem implementation
- **CompactStore** (0.5.31): read-only columnar store with per-label tables, double-indexed CSR adjacency, zone-map skip optimization, `CompactStoreBuilder` API. Integrates via `GrafeoDB::with_read_store()`
- **`compact()` method** (0.5.32): one-call conversion from live database to CompactStore. Available in Python, Node.js, WASM, C, Rust. ~60x memory reduction, 100x+ traversal speedup
- **Hybrid Logical Clock** (0.5.32): monotonic HLC timestamps in CDC events for causal ordering
- **Session CDC** (0.5.32): mutations via query sessions (`INSERT`, `SET`, `DELETE`) now generate CDC events, buffered per-transaction
- **Correctness hardening** (0.5.32): epoch monotonicity guarantees, concurrent stress tests, write-write conflict detection improvements
- **grafeo-server replication** (0.5.32): atomic sync apply (all-or-nothing transaction visibility on replicas), persistent replica epoch tracking (replicas resume from last position after restart), CDC auto-activation on replication primaries, relaxed replica guard for sync endpoints

### What's left in 0.5

- Crash recovery testing (failpoint injection), Jepsen testing for grafeo-server replication
- Bug fixes, stability monitoring, performance regression gates

---

## Next: 0.6, Release Candidate

*No new major features. Bug fixes, community integrations, and quality of life.*

The scope is intentionally narrow:

- **Bug fixes** from real-world 0.5/6 usage
- **Performance tuning** informed by actual workloads, not synthetic benchmarks
- **API ergonomics** and documentation polish
- **Binary size and compile time** optimization
- **C FFI parity refactor**: expand grafeo-c to match Python/Node.js API surface, update downstream bindings

The goal is confidence: if something works in 0.6, it works in 1.0.

---

## 1.0: Stable

Semantic versioning commitment. Public API frozen. No breaking changes without a major version bump.

---

## Future Considerations

Not scheduled, but on the radar:

- Distributed/clustered deployment
- Additional language bindings (Java/Kotlin, Swift)
- Cloud-native integrations

---

## Contributing

Interested in contributing? Check the [GitHub Issues](https://github.com/GrafeoDB/grafeo/issues) or join the [Discussions](https://github.com/orgs/GrafeoDB/discussions).

---

Last updated: April 2026
