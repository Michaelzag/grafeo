"""SQL/PGQ-specific pytest fixtures and configuration."""

import pytest

# Try to import grafeo
try:
    from grafeo import GrafeoDB

    GRAFEO_AVAILABLE = True
except ImportError:
    GRAFEO_AVAILABLE = False


def has_sql_pgq_support(db):
    """Check if SQL/PGQ support is available."""
    try:
        db.execute_sql("SELECT * FROM GRAPH_TABLE (MATCH (n) COLUMNS (n.name AS name))")
        return True
    except (AttributeError, NotImplementedError):
        return False
    except Exception:
        return True  # Has method but might fail for other reasons


@pytest.fixture
def db():
    """Create a fresh in-memory GrafeoDB instance.

    This fixture requires SQL/PGQ support.
    """
    if not GRAFEO_AVAILABLE:
        pytest.skip("grafeo not installed")
    database = GrafeoDB()
    if not has_sql_pgq_support(database):
        pytest.skip("SQL/PGQ support not available in this build")
    return database


@pytest.fixture
def social_db(db):
    """Create a database with a social network for SQL/PGQ tests.

    Structure:
    - Alice (Person, age 30, city NYC) -KNOWS-> Bob (since 2020)
    - Bob (Person, age 25, city LA) -KNOWS-> Charlie (since 2021)
    - Alice -KNOWS-> Charlie (since 2019)
    - Charlie (Person, age 35, city NYC)
    - Acme Corp (Company, founded 2010)
    - Globex Inc (Company, founded 2015)
    - Alice -WORKS_AT-> Acme Corp (role Engineer)
    - Bob -WORKS_AT-> Globex Inc (role Manager)
    - Charlie -WORKS_AT-> Acme Corp (role Director)
    """
    alice = db.create_node(["Person"], {"name": "Alice", "age": 30, "city": "NYC"})
    bob = db.create_node(["Person"], {"name": "Bob", "age": 25, "city": "LA"})
    charlie = db.create_node(["Person"], {"name": "Charlie", "age": 35, "city": "NYC"})

    acme = db.create_node(["Company"], {"name": "Acme Corp", "founded": 2010})
    globex = db.create_node(["Company"], {"name": "Globex Inc", "founded": 2015})

    db.create_edge(alice.id, bob.id, "KNOWS", {"since": 2020})
    db.create_edge(bob.id, charlie.id, "KNOWS", {"since": 2021})
    db.create_edge(alice.id, charlie.id, "KNOWS", {"since": 2019})

    db.create_edge(alice.id, acme.id, "WORKS_AT", {"role": "Engineer"})
    db.create_edge(bob.id, globex.id, "WORKS_AT", {"role": "Manager"})
    db.create_edge(charlie.id, acme.id, "WORKS_AT", {"role": "Director"})

    return db


@pytest.fixture
def chain_db(db):
    """Create a chain graph for path tests: A -> B -> C -> D.

    All nodes are Person type with KNOWS edges.
    """
    a = db.create_node(["Person"], {"name": "A"})
    b = db.create_node(["Person"], {"name": "B"})
    c = db.create_node(["Person"], {"name": "C"})
    d = db.create_node(["Person"], {"name": "D"})

    db.create_edge(a.id, b.id, "KNOWS", {})
    db.create_edge(b.id, c.id, "KNOWS", {})
    db.create_edge(c.id, d.id, "KNOWS", {})

    return db
