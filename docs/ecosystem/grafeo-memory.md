---
title: grafeo-memory
description: AI memory layer for LLM applications, powered by GrafeoDB.
---

# grafeo-memory

AI memory layer for LLM applications. Extract facts, entities, and relations from conversations and persist them in a GrafeoDB graph with vector embeddings for semantic search.

[:octicons-mark-github-16: GitHub](https://github.com/GrafeoDB/grafeo-memory){ .md-button }
[:material-package-variant: PyPI](https://pypi.org/project/grafeo-memory/){ .md-button }

## Overview

grafeo-memory provides a `MemoryManager` that orchestrates an **extract -> search -> reconcile -> execute** loop:

1. **Extract** facts, entities, and relations from text using an LLM
2. **Search** existing memories for duplicates or conflicts
3. **Reconcile** via LLM to decide ADD / UPDATE / DELETE / NONE
4. **Execute** mutations against the GrafeoDB graph

This keeps a persistent, deduplicated memory graph that grows and evolves over conversations.

## Installation

```bash
uv add grafeo-memory
# or
pip install grafeo-memory
```

Requires Python 3.12+, grafeo >= 0.5.1, and pydantic-ai.

## Quick Start

```python
from openai import OpenAI
from grafeo_memory import MemoryManager, MemoryConfig, OpenAIEmbedder

embedder = OpenAIEmbedder(OpenAI())
config = MemoryConfig(db_path="./memory.db", user_id="alice")

with MemoryManager("openai:gpt-4o-mini", config, embedder=embedder) as memory:
    # Add memories from conversations
    memory.add("I work at Acme Corp as a data scientist")
    memory.add("My favorite language is Python")

    # Semantic search
    results = memory.search("Where does the user work?")
    for r in results:
        print(r.text, r.score)

    # Update existing memory (reconciliation detects overlap)
    memory.add("I switched to a machine learning engineer role at Acme")

    # Get all memories
    all_memories = memory.get_all()
```

## Features

### Memory Management

- **Automatic deduplication** via LLM-powered reconciliation
- **Semantic search** using vector embeddings (HNSW index)
- **Multi-user support** with `user_id` isolation
- **Change history** via GrafeoDB's change data capture
- **Persistent or in-memory** storage modes

### Graph Structure

Memories are stored as a rich graph:

- `:Memory` nodes with `content`, `embedding`, and metadata properties
- `:Entity` nodes extracted from text (people, organizations, places, etc.)
- `:HAS_ENTITY` edges linking memories to their entities
- `:RELATION` edges between entities (e.g., "works at", "knows")

### LLM Integration

- **pydantic-ai** model strings for any supported provider (OpenAI, Anthropic, Mistral, Groq, Google)
- **Protocol-based** `EmbeddingClient` for custom embedding providers
- Structured extraction and reconciliation via pydantic-ai Agents

## API Reference

### MemoryManager

```python
MemoryManager(
    model: str,                        # pydantic-ai model string, e.g. "openai:gpt-4o-mini"
    config: MemoryConfig | None = None,
    *,
    embedder: EmbeddingClient,
)
```

Methods:

- `add(text, user_id=None, session_id=None, metadata=None) -> list[MemoryEvent]` - Extract and store memories
- `search(query, user_id=None, k=10) -> list[SearchResult]` - Semantic + graph search
- `get_all(user_id=None) -> list[SearchResult]` - Retrieve all memories
- `delete(memory_id) -> bool` - Delete a memory
- `delete_all(user_id=None) -> int` - Delete all memories for a user
- `history(memory_id) -> list[dict]` - Get change history (requires CDC feature)

### MemoryConfig

```python
MemoryConfig(
    db_path: str | None = None,          # None for in-memory
    user_id: str = "default",            # Default user scope
    embedding_dimensions: int = 1536,    # Embedding dimensions
    similarity_threshold: float = 0.7,   # Reconciliation threshold
)
```

## Requirements

- Python 3.12+
- grafeo >= 0.5.1
- pydantic-ai-slim

## License

Apache-2.0
