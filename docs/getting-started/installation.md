---
title: Installation
description: Install Grafeo for Python, Node.js, Go, Rust, or WebAssembly.
---

# Installation

Grafeo supports Python, Node.js/TypeScript, Go, Rust, and WebAssembly. Choose the installation method for your preferred language.

## Python

### Using uv (Recommended)

[uv](https://github.com/astral-sh/uv) is a fast Python package installer:

```bash
uv add grafeo
```

### Using pip (alternative)

```bash
pip install grafeo  # If uv is not available
```

### Verify Installation

```python
import grafeo

# Print version
print(grafeo.__version__)

# Create a test database
db = grafeo.GrafeoDB()
print("Grafeo installed successfully!")
```

### Platform Support

| Platform | Architecture | Support |
|----------|--------------|---------|
| Linux    | x86_64       | :material-check: Full |
| Linux    | aarch64      | :material-check: Full |
| macOS    | x86_64       | :material-check: Full |
| macOS    | arm64 (M1/M2)| :material-check: Full |
| Windows  | x86_64       | :material-check: Full |

## Node.js / TypeScript

```bash
npm install @grafeo-db/js
```

### Verify Installation

```js
const { GrafeoDB } = require('@grafeo-db/js');

const db = await GrafeoDB.create();
console.log('Grafeo installed successfully!');
await db.close();
```

## Go

```bash
go get github.com/GrafeoDB/grafeo/crates/bindings/go
```

### Verify Installation

```go
package main

import (
    "fmt"
    grafeo "github.com/GrafeoDB/grafeo/crates/bindings/go"
)

func main() {
    db, _ := grafeo.OpenInMemory()
    defer db.Close()
    fmt.Println("Grafeo installed successfully!")
}
```

## WebAssembly

```bash
npm install @grafeo-db/wasm
```

### Verify Installation

```js
import init, { Database } from '@grafeo-db/wasm';

await init();
const db = new Database();
console.log('Grafeo WASM installed successfully!');
```

## Rust

### Using Cargo

Add Grafeo to your project:

```bash
cargo add grafeo
```

Or add it manually to your `Cargo.toml`:

```toml
[dependencies]
grafeo = "0.4"
```

### Feature Flags

All query languages and AI features are enabled by default via the `full` feature group. Use feature groups or individual flags to customize:

```toml
[dependencies]
# Default: all languages + AI features
grafeo = "0.5"

# Only query languages, no AI features
grafeo = { version = "0.5", default-features = false, features = ["languages"] }

# GQL with AI features
grafeo = { version = "0.5", default-features = false, features = ["gql", "ai"] }

# Minimal: GQL only
grafeo = { version = "0.5", default-features = false, features = ["gql"] }

# With ONNX embedding generation (opt-in, not in full)
grafeo = { version = "0.5", features = ["embed"] }
```

#### Feature Groups

| Group | Contents | Description |
|-------|----------|-------------|
| `full` | languages + ai | Everything (default) |
| `languages` | gql, cypher, sparql, gremlin, graphql, sql-pgq | All query language parsers |
| `ai` | vector-index, text-index, hybrid-search, cdc | AI/RAG search + change tracking |
| `embed` | ort, tokenizers | ONNX embedding generation (opt-in, ~17MB) |

#### Individual Language Flags

| Feature | Description |
|---------|-------------|
| `gql` | GQL (ISO/IEC 39075) — default query language |
| `cypher` | Cypher (openCypher 9.0) |
| `sparql` | SPARQL (W3C 1.1) + RDF support |
| `gremlin` | Gremlin (Apache TinkerPop) |
| `graphql` | GraphQL |
| `sql-pgq` | SQL/PGQ (SQL:2023 GRAPH_TABLE) |

#### Individual AI Flags

| Feature | Description |
|---------|-------------|
| `vector-index` | HNSW approximate nearest neighbor index |
| `text-index` | BM25 inverted index for full-text search |
| `hybrid-search` | Combined text + vector search with score fusion |
| `cdc` | Change data capture (before/after property snapshots) |

### Verify Installation

```rust
use grafeo::Database;

fn main() -> Result<(), grafeo::Error> {
    let db = Database::open_in_memory()?;
    println!("Grafeo installed successfully!");
    Ok(())
}
```

## Grafeo Server (Docker)

For a standalone database server accessible via REST API, use [grafeo-server](../ecosystem/grafeo-server.md):

```bash
# Standard — all query languages, AI/search, web UI
docker run -p 7474:7474 grafeo/grafeo-server
```

Three image variants are available:

| Variant | Tag | Description |
|---------|-----|-------------|
| **lite** | `grafeo-server:lite` | GQL only, no UI — smallest footprint |
| **standard** | `grafeo-server:latest` | All languages + AI/search + web UI |
| **full** | `grafeo-server:full` | Everything + auth + TLS + ONNX embed |

```bash
# Lite — minimal, GQL only
docker run -p 7474:7474 grafeo/grafeo-server:lite

# Full — production with auth and TLS
docker run -p 7474:7474 grafeo/grafeo-server:full \
  --auth-token my-secret --data-dir /data
```

Server at `http://localhost:7474`. Web UI (standard/full) at `http://localhost:7474/studio/`.

See the [grafeo-server documentation](../ecosystem/grafeo-server.md) for full API reference and configuration.

## Building from Source

### Clone the Repository

```bash
git clone https://github.com/GrafeoDB/grafeo.git
cd grafeo
```

### Build Rust Crates

```bash
cargo build --workspace --release
```

### Build Python Package

```bash
cd crates/bindings/python
uv add maturin
maturin develop --release
```

### Build Node.js Package

```bash
cd crates/bindings/node
npm install
npm run build
```

### Build WASM Package

```bash
wasm-pack build crates/bindings/wasm --target web --release
```

## Next Steps

Now that you have Grafeo installed, continue to the [Quick Start](quickstart.md) guide.
