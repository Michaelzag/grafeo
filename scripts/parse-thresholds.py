#!/usr/bin/env python3
"""Parse bench-thresholds.toml and output TSV for bench-compare.sh.

Usage:
    python scripts/parse-thresholds.py bench-thresholds.toml

Output (TSV):
    benchmark_pattern\tthreshold_pct\tfail_ci
    epoch_arena_*\t8\ttrue
    query_*\t12\ttrue
    ...
    __default__\t15\tfalse
"""

from __future__ import annotations

import fnmatch
import sys
import tomllib
from pathlib import Path


def main() -> None:
    if len(sys.argv) < 2:
        print(
            f"Usage: {sys.argv[0]} <thresholds.toml> [benchmark_name]", file=sys.stderr
        )
        sys.exit(1)

    config_path = Path(sys.argv[1])
    if not config_path.exists():
        print(f"Error: {config_path} not found", file=sys.stderr)
        sys.exit(1)

    with config_path.open("rb") as f:
        config = tomllib.load(f)

    defaults = config.get("defaults", {})
    default_threshold = defaults.get("threshold_pct", 15)
    default_fail_ci = defaults.get("fail_ci", False)

    categories = config.get("categories", {})

    # If a benchmark name is given, resolve its threshold and exit.
    if len(sys.argv) >= 3:
        bench_name = sys.argv[2]
        threshold, fail_ci = resolve(
            bench_name, categories, default_threshold, default_fail_ci
        )
        print(f"{bench_name}\t{threshold}\t{str(fail_ci).lower()}")
        return

    # Otherwise, dump every pattern as TSV.
    print("benchmark_pattern\tthreshold_pct\tfail_ci")
    for _name, cat in categories.items():
        threshold = cat.get("threshold_pct", default_threshold)
        fail_ci = cat.get("fail_ci", default_fail_ci)
        for pattern in cat.get("benchmarks", []):
            print(f"{pattern}\t{threshold}\t{str(fail_ci).lower()}")
    print(f"__default__\t{default_threshold}\t{str(default_fail_ci).lower()}")


def resolve(
    bench_name: str,
    categories: dict,
    default_threshold: int,
    default_fail_ci: bool,
) -> tuple[int, bool]:
    """Return (threshold_pct, fail_ci) for a specific benchmark name."""
    for _name, cat in categories.items():
        for pattern in cat.get("benchmarks", []):
            if fnmatch.fnmatch(bench_name, pattern):
                return cat.get("threshold_pct", default_threshold), cat.get(
                    "fail_ci", default_fail_ci
                )
    return default_threshold, default_fail_ci


if __name__ == "__main__":
    main()
