"""Cypher-specific pytest fixtures and configuration."""

import os

import pytest
from grafeo import GrafeoDB

_db = GrafeoDB()
HAS_CYPHER = hasattr(_db, "execute_cypher")

_THIS_DIR = os.path.dirname(__file__)


def pytest_collection_modifyitems(config, items):
    """Skip Cypher tests when the feature is not compiled in."""
    if HAS_CYPHER:
        return
    skip = pytest.mark.skip(reason="grafeo built without cypher feature")
    for item in items:
        if str(item.fspath).startswith(_THIS_DIR):
            item.add_marker(skip)
