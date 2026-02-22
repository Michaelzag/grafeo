---
title: Cost Model
description: Query cost estimation.
tags:
  - architecture
  - optimization
---

# Cost Model

The cost model estimates execution cost for plan selection.

## Cost Components

| Component | Weight | Description |
|-----------|--------|-------------|
| CPU | 1.0 | Computation cost |
| I/O | 10.0 | Disk access cost |
| Memory | 0.5 | Memory allocation |
| Network | 100.0 | Data transfer (future) |

## Cost Formula

```
Total Cost = CPU_cost * cpu_weight
           + IO_cost * io_weight
           + Mem_cost * mem_weight
```

## Operator Costs

| Operator | Cost Formula |
|----------|--------------|
| Scan | rows * column_count |
| Filter | input_rows * selectivity |
| Hash Join | build_rows + probe_rows |
| Sort | rows * log(rows) |

## Statistics-Driven Estimation (0.5.8+)

!!! note "Improved in 0.5.8"
    As of version 0.5.8, the cost model uses real fanout derived from graph statistics
    (average degree, label cardinalities, edge-type frequencies) instead of hardcoded
    defaults. This leads to significantly better plan selection for traversal-heavy queries,
    especially on graphs with skewed degree distributions.
