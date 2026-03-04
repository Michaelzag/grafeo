---
title: Mutations
description: Creating, updating and deleting graph data in GQL.
tags:
  - gql
  - mutations
---

# Mutations

GQL supports mutations for creating, updating and deleting graph data.

## Creating Nodes

```sql
-- Create a node
INSERT (:Person {name: 'Alice', age: 30})

-- Create multiple nodes
INSERT (:Person {name: 'Alice'})
INSERT (:Person {name: 'Bob'})

-- Create with multiple labels
INSERT (:Person:Employee {name: 'Carol'})
```

## Creating Edges

```sql
-- Create an edge between existing nodes
MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'})
INSERT (a)-[:KNOWS]->(b)

-- Create edge with properties
MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'})
INSERT (a)-[:KNOWS {since: 2020, strength: 'close'}]->(b)
```

## Updating Properties

```sql
-- Set a property
MATCH (p:Person {name: 'Alice'})
SET p.age = 31

-- Set multiple properties
MATCH (p:Person {name: 'Alice'})
SET p.age = 31, p.city = 'New York'

-- Set from another property
MATCH (p:Person)
SET p.displayName = p.firstName + ' ' + p.lastName
```

## Removing Properties

```sql
-- Remove a property
MATCH (p:Person {name: 'Alice'})
REMOVE p.temporaryField

-- Set to null (equivalent)
MATCH (p:Person {name: 'Alice'})
SET p.temporaryField = null
```

## Map Property Operations

### Replace All Properties

`SET n = {map}` replaces all existing properties with the map contents:

```sql
-- Replace all properties on a node
MATCH (p:Person {name: 'Alice'})
SET p = {name: 'Alice', age: 31, city: 'NYC'}
-- Any properties not in the map are removed
```

### Merge Properties

`SET n += {map}` merges the map into existing properties, keeping properties not in the map:

```sql
-- Add or update properties, keep existing ones
MATCH (p:Person {name: 'Alice'})
SET p += {city: 'NYC', role: 'engineer'}
-- Existing properties like name and age are preserved
```

## Label Operations

```sql
-- Add labels to a node
MATCH (p:Person {name: 'Alice'})
SET p:Employee:Manager

-- Remove a label
MATCH (p:Person {name: 'Alice'})
REMOVE p:Manager
```

## Deleting Nodes

GQL supports two delete modes for nodes:

```sql
-- DELETE (or NODETACH DELETE): errors if the node has edges
MATCH (p:Person {name: 'Alice'})
DELETE p

-- Explicit NODETACH (same behavior as bare DELETE)
MATCH (p:Person {name: 'Alice'})
NODETACH DELETE p

-- DETACH DELETE: delete the node and all its connected edges
MATCH (p:Person {name: 'Alice'})
DETACH DELETE p
```

!!! tip
    Use `DELETE` (without DETACH) when you want to ensure no edges are accidentally removed. The query will fail if the node still has connections, giving you a chance to handle them explicitly.

## Deleting Edges

```sql
-- Delete specific edge
MATCH (a:Person {name: 'Alice'})-[r:KNOWS]->(b:Person {name: 'Bob'})
DELETE r

-- Delete all edges of a type from a node
MATCH (p:Person {name: 'Alice'})-[r:KNOWS]->()
DELETE r
```

## UNWIND (List Expansion)

Expand a list into individual rows. Useful for batch operations.

```sql
-- Unwind a literal list
UNWIND [1, 2, 3] AS x
RETURN x

-- Unwind with parameters (Python: db.execute(query, {'names': ['Alice', 'Bob']}))
UNWIND $names AS name
RETURN name

-- Batch create edges from a parameter list
UNWIND $edges AS e
MATCH (a:Person {name: e.from}), (b:Person {name: e.to})
INSERT (a)-[:KNOWS]->(b)
```

## FOR (GQL Standard List Iteration)

The GQL standard equivalent of UNWIND (ISO/IEC 39075 section 14.8).

```sql
-- Iterate over a list
FOR x IN [4, 5, 6]
RETURN x

-- Batch insert nodes
FOR person IN $people
INSERT (:Person {name: person.name, age: person.age})
```

### FOR WITH ORDINALITY / OFFSET

Track the position of each element during iteration:

```sql
-- 1-based index (ORDINALITY)
FOR x IN ['a', 'b', 'c'] WITH ORDINALITY i
RETURN x, i
-- Returns: ('a', 1), ('b', 2), ('c', 3)

-- 0-based index (OFFSET)
FOR x IN ['a', 'b', 'c'] WITH OFFSET i
RETURN x, i
-- Returns: ('a', 0), ('b', 1), ('c', 2)
```

## Merge (Upsert)

```sql
-- Create if not exists, match if exists
MERGE (p:Person {email: 'alice@example.com'})
SET p.lastSeen = timestamp()
RETURN p

-- Merge with different actions
MERGE (p:Person {email: 'alice@example.com'})
ON CREATE SET p.created = timestamp()
ON MATCH SET p.lastSeen = timestamp()
RETURN p
```
