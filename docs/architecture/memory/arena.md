---
title: Arena Allocators
description: Epoch-based arena allocation.
tags:
  - architecture
  - memory
---

# Arena Allocators

Arenas provide fast, bulk allocation for query execution.

## How Arenas Work

```
Arena:
├── Chunk 1 (4KB): [allocated][allocated][free...]
├── Chunk 2 (4KB): [allocated][free.............]
└── Chunk 3 (4KB): [free......................]

Allocation: Bump pointer in current chunk
Deallocation: Reset entire arena at once
```

## Benefits

- **Fast allocation** - Just bump a pointer
- **No fragmentation** - All freed at once
- **Cache friendly** - Sequential allocation

## Usage

```rust
use grafeo_common::types::EpochId;

let arena = Arena::new(EpochId(0));

// Allocate within arena
let data = arena.alloc_value(node);
let more = arena.alloc_slice(&values);
```
