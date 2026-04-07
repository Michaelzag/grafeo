# Feature Profiles

## Motivation

Grafeo aims to be a universal graph database: property graphs, RDF, analytics, AI memory, browser, production server. But no single user needs all of that. Feature profiles let every user get exactly what they need, with zero overhead from what they don't.

There are two layers:

- **Layer 1: Profiles**, named groups consistent across the entire ecosystem. This is what most users interact with.
- **Layer 2: Atoms**, individual feature flags for power users who want precise control. Profiles are composed from these.

## Current Profiles

The current system uses deployment-target names:

| Profile | Contents | Use case |
| --- | --- | --- |
| `embedded` | gql, ai, algos, parallel, regex, grafeo-file | Python, Node.js, C, MCP, in-process |
| `browser` | gql, regex-lite | WASM, grafeo-web |
| `server` | embedded, languages, storage, rdf, cdc, async-storage, tracing | grafeo-server |
| `full` | server | Everything (except embed) |

**Defaults**: grafeo facade and bindings use `embedded`. WASM uses `browser`. CLI uses `gql` + storage.

## Proposed Profiles

The proposed system replaces deployment-target names with persona-driven names that describe *what you're building*, not *where it runs*.

| Profile | Persona | What it enables |
| --- | --- | --- |
| **LPG** | Graph App Developer | GQL, Cypher, Gremlin, SQL/PGQ, storage |
| **RDF** | Knowledge Engineer | SPARQL, GraphQL, RDF store, ring-index, storage |
| **Analytics** | Data Scientist | Algorithms, vector/text/hybrid search, parquet + jsonl import |
| **AI** | AI Memory / Agent Developer | Temporal, CDC, vector/text/hybrid search |
| **Edge** | Frontend / Edge Developer | GQL only, compact-store, regex-lite (standalone, minimal) |
| **Enterprise** | Platform Operator | Auth, TLS, metrics, tracing, sync, replication, push-changefeed (grafeo-server only) |

### Composition Rules

- **Model profiles** (LPG, RDF): pick one or both. These are the foundation.
- **Capability profiles** (Analytics, AI, Enterprise): stack on top of a model profile.
- **Constrained profile** (Edge): minimal by default, composable with constraints. Users can add atoms like `algos` if they accept the size increase.

Examples:

```toml
# AI memory developer
grafeo = { features = ["lpg", "ai"] }

# Semantic data scientist
grafeo = { features = ["rdf", "analytics"] }

# Full production stack (grafeo-server only, enterprise is server-scoped)
# grafeo-server = { features = ["lpg", "rdf", "ai", "enterprise"] }

# Browser app
grafeo = { features = ["edge"] }

# Power user: just Cypher + vector search
grafeo = { features = ["cypher", "vector-index", "wal"] }
```

## Profile Definitions

### LPG

```toml
lpg = ["gql", "cypher", "gremlin", "sql-pgq", "storage", "regex"]
```

All Labeled Property Graph query languages plus persistence. The default choice for application developers working with nodes, edges, labels, and properties.

### RDF

```toml
rdf = ["sparql", "graphql", "triple-store", "ring-index", "storage", "regex"]
```

Full RDF triple store with SPARQL, space-efficient indexing, and persistence. For knowledge engineers working with ontologies and linked data. The `triple-store` atom enables the storage layer, and `ring-index` adds compact indexing (which pulls in `succinct-indexes` automatically).

> **Note:** `owl-schema` and `rdfs-schema` currently only exist in grafeo-server. Promoting them to the engine level for this profile is an open question.

### Analytics

```toml
analytics = ["algos", "vector-index", "text-index", "hybrid-search", "jsonl-import", "parquet-import"]
```

27 graph algorithms (PageRank, Louvain, SSSP, Dijkstra, BFS/DFS, centrality, community detection, MST, flow, isomorphism, structural analysis, clustering), search indexes, and data import. Combine with LPG or RDF depending on the dataset.

### AI

```toml
ai = ["temporal", "cdc", "vector-index", "text-index", "hybrid-search"]
```

Structured memory for LLMs, agents, and RAG pipelines. Temporal history tracks how the graph evolves, CDC enables change feeds, and search indexes support vector/text retrieval.

> **Note:** `embed` (ONNX embedding generation, ~17 MB) is deliberately excluded from this profile. Most AI memory use cases (grafeo-memory, MCP, LangChain) bring embeddings via API calls. Opt in explicitly with `features = ["ai", "embed"]` if you need in-process embedding.
>
> This redefines the current `ai` group. Today, `ai` = `["vector-index", "text-index", "hybrid-search", "cdc"]` at the engine level. The proposed definition adds `temporal` and removes `cdc`'s implicit inclusion (it becomes explicit).

### Edge

```toml
edge = ["gql", "compact-store", "regex-lite"]
```

Minimal profile for browser, mobile, and resource-constrained environments. Compact store for pre-built read-only datasets, lightweight regex, smallest possible binary (~500 KB gzipped for WASM).

### Enterprise

```toml
enterprise = ["auth", "tls", "metrics", "tracing", "sync", "replication", "push-changefeed"]
```

Production operations: authentication, encryption in transit, observability, and data replication.

> **Important:** This profile only applies to **grafeo-server**. Auth, TLS, sync, replication, and push-changefeed are defined exclusively in the grafeo-server workspace. `metrics` and `tracing` are available as individual atoms in the engine, but the `enterprise` umbrella is server-scoped. On grafeo-server, this additionally enables all transport layers (HTTP, GWP, Bolt, Studio).

## Migration from Current Profiles

| Old Profile | New Equivalent | Notes |
| --- | --- | --- |
| `embedded` | `lpg` + `ai` + `algos` + `parallel` | Current default for Python/Node/C |
| `browser` | `edge` | Current default for WASM |
| `server` | `lpg` + `rdf` + `ai` + `enterprise` | Approximate |
| `full` | `lpg` + `rdf` + `analytics` + `ai` | Everything except enterprise/embed |

**Deprecation path:**

1. Add new profile names as aliases alongside old names
2. Emit `cfg` deprecation warnings for old names
3. Update all binding defaults to use new names
4. Remove old names after one major version

**Default feature question:** What should `grafeo = { version = "..." }` give you? Currently it maps to `embedded`. Under the new system, the default needs to be decided (likely `lpg`).

## Ecosystem Matrix

The profile names are consistent across every project. The table below shows which profiles are available in each project, either as configurable feature flags or as the project's inherent profile alignment.

### Core Engine

| Project | LPG | RDF | Analytics | AI | Edge | Enterprise | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **grafeo** (engine) | flag | flag | flag | flag | flag | n/a | All profiles except Enterprise |
| **grafeo-server** | flag | flag | flag | flag | n/a | flag | Enterprise is server-only |
| **grafeo-cli** | flag | flag | flag | flag | n/a | n/a | Interactive REPL and CLI tooling |

### Language Bindings

| Project | LPG | RDF | Analytics | AI | Edge | Enterprise | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Python** (grafeo-py) | flag | flag | flag | flag | n/a | n/a | Default: `lpg` |
| **Node.js** (grafeo-node) | flag | flag | flag | flag | n/a | n/a | Default: `lpg` |
| **WASM** (grafeo-wasm) | flag | flag | flag | n/a | flag (default) | n/a | Edge is default |
| **C** (grafeo-c) | flag | flag | flag | flag | n/a | n/a | Bridge for C#, Dart, Go |
| **C#** | via C | via C | via C | via C | n/a | n/a | Feature selection at C build time |
| **Dart** | via C | via C | via C | via C | n/a | n/a | Feature selection at C build time |
| **Go** | via C | via C | via C | via C | n/a | n/a | Feature selection at C build time |

### AI / Agent Ecosystem

| Project | LPG | RDF | Analytics | AI | Edge | Enterprise | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **grafeo-memory** | inherent | - | - | inherent | - | - | AI memory layer |
| **grafeo-langchain** | inherent | - | - | inherent | - | - | LangChain integration |
| **grafeo-llamaindex** | inherent | - | - | inherent | - | - | LlamaIndex integration |
| **grafeo-mcp** | inherent | - | - | inherent | - | - | MCP server for AI agents |

### Web / Visualization

| Project | LPG | RDF | Analytics | AI | Edge | Enterprise | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **grafeo-web** | - | - | - | - | inherent | - | WASM in browser |
| **playground** | - | - | - | - | inherent | - | Interactive graph playground |
| **anywidget-graph** | inherent | - | - | - | - | - | Notebook graph visualization |
| **anywidget-vector** | - | - | - | inherent | - | - | Notebook vector visualization |

### Protocol Libraries

| Project | LPG | RDF | Analytics | AI | Edge | Enterprise | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **boltr** | - | - | - | - | - | inherent | Bolt v5 protocol |
| **gwp** | - | - | - | - | - | inherent | GQL Wire Protocol (gRPC) |

### Accelerators and Tooling

| Project | LPG | RDF | Analytics | AI | Edge | Enterprise | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **grafeo-cuda** | - | - | inherent | - | - | - | GPU-accelerated algorithms |
| **graph-bench** | all | all | all | all | - | - | Benchmark suite |

### Legend

- **flag**: Profile is available as a configurable feature flag. User opts in.
- **inherent**: The project is inherently aligned with this profile. No flag needed.
- **via C**: Feature selection happens at C binding compile time, propagates to higher-level bindings.
- **n/a**: Profile does not apply to this project.
- **-**: Not applicable or not supported.

## Atom Reference

The complete list of individual feature flags (Layer 2) that profiles are composed from. Status indicates whether the atom exists in the codebase today.

### Query Languages

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `gql` | LPG, Edge | ISO/IEC GQL standard | Implemented |
| `cypher` | LPG | openCypher 9.0 | Implemented |
| `sparql` | RDF | W3C SPARQL 1.1 | Implemented |
| `gremlin` | LPG | Apache TinkerPop | Implemented |
| `graphql` | RDF | GraphQL over RDF | Implemented |
| `sql-pgq` | LPG | SQL:2023 GRAPH_TABLE | Implemented |

### Storage

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `storage` | LPG, RDF | Umbrella: WAL + grafeo-file + spill + mmap | Implemented |
| `wal` | (storage) | Write-ahead log persistence | Implemented |
| `grafeo-file` | (storage) | Single-file .grafeo format | Implemented |
| `spill` | (storage) | Out-of-core disk spilling | Implemented |
| `mmap` | (storage) | Memory-mapped file storage | Implemented |
| `async-storage` | (standalone) | Async WAL backend (tokio) | Implemented |
| `compact-store` | Edge | Read-only columnar store | Implemented |

### Graph Model

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `triple-store` | RDF | RDF triple store with 6-way indexing (currently `rdf` in Cargo.toml, renamed) | Implemented |
| `ring-index` | RDF | Space-efficient RDF index (requires succinct-indexes) | Implemented |
| `succinct-indexes` | (pulled in by ring-index) | Rank/select bitvectors, Elias-Fano, wavelet trees | Implemented |
| `owl-schema` | RDF (server only) | OWL schema loading | Server only |
| `rdfs-schema` | RDF (server only) | RDFS schema support | Server only |

### Search and AI

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `vector-index` | Analytics, AI | HNSW approximate nearest neighbor | Implemented |
| `text-index` | Analytics, AI | BM25 inverted index | Implemented |
| `hybrid-search` | Analytics, AI | Combined vector + text search | Implemented |
| `embed` | (standalone) | ONNX embedding generation (~17 MB overhead) | Implemented |
| `algos` | Analytics | 27 graph algorithms | Implemented |

### Temporal and Change Tracking

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `temporal` | AI | Append-only versioned properties | Implemented |
| `cdc` | AI | Change data capture with history API | Implemented |

### Import

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `jsonl-import` | Analytics | JSON Lines file import | Implemented |
| `parquet-import` | Analytics | Apache Parquet import | Implemented |

### Execution

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `parallel` | (standalone) | Morsel-driven parallelism (rayon) | Implemented |
| `block-stm` | (standalone) | Parallel batch transaction execution | Implemented |
| `tiered-storage` | (standalone) | Hot/cold version storage with epochs | Implemented |

### Operations (grafeo-server only)

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `auth` | Enterprise | Authentication provider | Server only |
| `tls` | Enterprise | TLS/HTTPS encryption | Server only |
| `metrics` | Enterprise | Lock-free query/transaction metrics | Implemented |
| `tracing` | Enterprise | Distributed tracing spans | Implemented |
| `sync` | Enterprise | Pull-based changefeed for offline-first | Server only |
| `push-changefeed` | Enterprise | Push-based SSE/WebSocket changefeed | Server only |
| `replication` | Enterprise | Primary-replica replication | Server only |

### Transports (grafeo-server only)

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `http` | Enterprise | HTTP/REST + OpenAPI + WebSocket | Server only |
| `gwp` | Enterprise | GQL Wire Protocol (gRPC) | Server only |
| `bolt` | Enterprise | Bolt v5 (Neo4j driver compat) | Server only |
| `studio` | Enterprise | Embedded web UI | Server only |

### Regex

| Atom | Profile | Description | Status |
| --- | --- | --- | --- |
| `regex` | LPG, RDF | Full regex engine | Implemented |
| `regex-lite` | Edge | Lightweight regex for WASM | Implemented |
