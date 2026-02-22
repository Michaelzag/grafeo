---
title: Cardinality Estimation
description: Estimating result set sizes.
tags:
  - architecture
  - optimization
---

# Cardinality Estimation

Accurate cardinality estimation is crucial for plan selection.

## Statistics Collected

| Statistic | Purpose |
|-----------|---------|
| Element count | Base cardinality (nodes/edges per label) |
| Distinct count | Join estimation |
| Histograms | Range selectivity |
| Null fraction | Null handling |

## Selectivity Estimation

```
// Equality predicate
selectivity = 1 / distinct_count

// Range predicate
selectivity = (high - low) / (max - min)

// Join
output_rows = (rows_a * rows_b) / max(distinct_a, distinct_b)
```

## Statistics Collection

Statistics are collected automatically by the query engine during graph operations.
Grafeo tracks per-label and per-property statistics for cardinality estimation.
