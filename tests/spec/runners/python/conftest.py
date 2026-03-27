"""pytest plugin that discovers and runs .gtest spec files through the Python bindings.

Drop this file (or the containing ``runners/python`` package) on the pytest
search path and every ``.gtest`` file under ``tests/spec/`` becomes a set of
pytest test items.

Each test case gets a **fresh** ``GrafeoDB()`` instance, loads the declared
dataset, runs setup queries, executes the query, and asserts the expected
result.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Dict, List, Optional

import pytest

# Make sure the runner package is importable
_runner_dir = Path(__file__).resolve().parent
if str(_runner_dir) not in sys.path:
    sys.path.insert(0, str(_runner_dir))

from parser import GtestFile, TestCase, parse_gtest_file  # noqa: E402
from comparator import (  # noqa: E402
    assert_count,
    assert_empty,
    assert_error,
    assert_hash,
    assert_rows_ordered,
    assert_rows_sorted,
)

# ---------------------------------------------------------------------------
# Grafeo availability
# ---------------------------------------------------------------------------

try:
    import grafeo  # noqa: F401

    GRAFEO_AVAILABLE = True
except ImportError:
    GRAFEO_AVAILABLE = False

# Languages that require a separate execute method
_LANGUAGE_METHODS: Dict[str, str] = {
    "gql": "execute",
    "cypher": "execute_cypher",
    "gremlin": "execute_gremlin",
    "graphql": "execute_graphql",
    "sparql": "execute_sparql",
    "sql-pgq": "execute_sql",
    "sql_pgq": "execute_sql",
}

# Repo root: tests/spec/runners/python/conftest.py -> five .parent calls
_REPO_ROOT = Path(__file__).resolve().parent.parent.parent.parent.parent


# ---------------------------------------------------------------------------
# pytest hooks
# ---------------------------------------------------------------------------


def pytest_collect_file(parent, file_path):
    """Discover .gtest files for collection."""
    if file_path.suffix == ".gtest":
        return GtestFileCollector.from_parent(parent, path=file_path)
    return None


# ---------------------------------------------------------------------------
# Collector
# ---------------------------------------------------------------------------


class GtestFileCollector(pytest.File):
    """Collects test items from a single .gtest file."""

    def collect(self):
        try:
            gtest_file = parse_gtest_file(self.path)
        except Exception as exc:
            raise pytest.UsageError(f"Failed to parse {self.path}: {exc}") from exc

        for tc in gtest_file.tests:
            if tc.variants:
                # Rosetta: one item per variant language
                for lang, query in tc.variants.items():
                    item_name = f"{tc.name}[{lang}]"
                    yield GtestItem.from_parent(
                        self,
                        name=item_name,
                        gtest_file=gtest_file,
                        test_case=tc,
                        variant_lang=lang,
                        variant_query=query,
                    )
            else:
                yield GtestItem.from_parent(
                    self,
                    name=tc.name,
                    gtest_file=gtest_file,
                    test_case=tc,
                    variant_lang=None,
                    variant_query=None,
                )


# ---------------------------------------------------------------------------
# Test item
# ---------------------------------------------------------------------------


class GtestItem(pytest.Item):
    """A single test case from a .gtest file."""

    def __init__(
        self,
        name: str,
        parent: GtestFileCollector,
        gtest_file: GtestFile,
        test_case: TestCase,
        variant_lang: Optional[str],
        variant_query: Optional[str],
    ):
        super().__init__(name, parent)
        self.gtest_file = gtest_file
        self.test_case = test_case
        self.variant_lang = variant_lang
        self.variant_query = variant_query

    def runtest(self):
        tc = self.test_case
        meta = self.gtest_file.meta

        # Skip if grafeo is not importable
        if not GRAFEO_AVAILABLE:
            pytest.skip("grafeo Python package not installed")

        # Skip if test has a skip field
        if tc.skip is not None:
            pytest.skip(f"skipped in .gtest: {tc.skip}")

        # Determine language
        language = self.variant_lang or meta.language or "gql"

        # Check requires: skip if the binding does not expose the method
        all_requires = list(meta.requires)
        if language not in ("gql", "") and language not in all_requires:
            all_requires.append(language)

        for req in all_requires:
            method_name = _LANGUAGE_METHODS.get(req)
            if method_name and not hasattr(grafeo.GrafeoDB, method_name):
                pytest.skip(
                    f"grafeo build does not support '{req}' (no {method_name} method)"
                )

        # Fresh database per test
        db = grafeo.GrafeoDB()

        # Load dataset
        if meta.dataset and meta.dataset != "empty":
            _load_dataset(db, meta.dataset)

        # Run setup queries (always GQL)
        for setup_q in tc.setup:
            db.execute(setup_q)

        # Determine query / statements
        query = self.variant_query or tc.query
        statements = tc.statements

        if statements:
            queries = list(statements)
        elif query:
            queries = [query]
        else:
            pytest.fail(f"No query or statements in test '{tc.name}' in {self.path}")
            return  # unreachable, but keeps linters happy

        expect = tc.expect

        # Error tests
        if expect.error is not None:
            self._run_error_test(db, language, queries, expect.error)
            return

        # Execute all-but-last as fire-and-forget
        for q in queries[:-1]:
            _execute(db, language, q)

        # Last query: capture result
        result = _execute(db, language, queries[-1])

        # Assertions
        if expect.empty:
            assert_empty(result)
        elif expect.count is not None:
            assert_count(result, expect.count)
        elif expect.hash is not None:
            assert_hash(result, expect.hash)
        elif expect.rows:
            if expect.ordered:
                assert_rows_ordered(result, expect.rows)
            else:
                assert_rows_sorted(result, expect.rows)
        # If none of the above, the test just checks the query does not error

    def _run_error_test(
        self,
        db,
        language: str,
        queries: List[str],
        expected_substr: str,
    ) -> None:
        """Execute queries expecting the last one to raise an error."""
        # Execute all-but-last normally
        for q in queries[:-1]:
            _execute(db, language, q)

        # Last query should fail
        try:
            _execute(db, language, queries[-1])
        except Exception as exc:
            assert_error(exc, expected_substr)
        else:
            pytest.fail(
                f"Expected error containing '{expected_substr}' but query succeeded"
            )

    def repr_failure(self, excinfo):
        """Provide a readable failure message."""
        if isinstance(excinfo.value, AssertionError if False else AssertionError):
            return str(excinfo.value)
        return super().repr_failure(excinfo)

    def reportinfo(self):
        return self.path, None, f"{self.path.name}::{self.name}"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _load_dataset(db, dataset_name: str) -> None:
    """Load a .setup dataset file into the database using GQL."""
    dataset_path = _REPO_ROOT / "tests" / "spec" / "datasets" / f"{dataset_name}.setup"
    if not dataset_path.exists():
        pytest.fail(f"Dataset file not found: {dataset_path}")

    content = dataset_path.read_text(encoding="utf-8")
    for line in content.splitlines():
        trimmed = line.strip()
        if not trimmed or trimmed.startswith("#"):
            continue
        db.execute(trimmed)


def _execute(db, language: str, query: str):
    """Execute a query in the specified language, returning the QueryResult."""
    method_name = _LANGUAGE_METHODS.get(language, "execute")
    method = getattr(db, method_name, None)
    if method is None:
        pytest.skip(
            f"grafeo build does not support language '{language}' "
            f"(no {method_name} method)"
        )
    return method(query)
