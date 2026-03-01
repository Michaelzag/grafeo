"""GQL syntax compliance tests (ISO/IEC 39075:2024).

Each test targets a specific spec element with a minimal query.
Tests are organized by category: clauses, expressions, functions, patterns, predicates.
"""


# =============================================================================
# CLAUSES (sec 13-14)
# =============================================================================


class TestGqlClauses:
    """Test GQL clause types from sec 13-14."""

    # --- MATCH ---

    def test_match_by_label(self, db):
        """MATCH (n:Label) basic label filter."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        db.create_node(["Animal"], {"name": "Rex"})
        result = list(db.execute("MATCH (n:Person) RETURN n.name"))
        names = {r["n.name"] for r in result}
        assert names == {"Alice", "Bob"}

    def test_match_by_property(self, db):
        """MATCH with inline property filter."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        result = list(db.execute("MATCH (n:Person {name: 'Alice'}) RETURN n.age"))
        assert len(result) == 1
        assert result[0]["n.age"] == 30

    def test_multiple_match_clauses(self, db):
        """Two sequential MATCH clauses (cross product)."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Company"], {"name": "Acme"})
        result = list(
            db.execute("MATCH (p:Person) MATCH (c:Company) RETURN p.name, c.name")
        )
        assert len(result) == 1
        assert result[0]["p.name"] == "Alice"
        assert result[0]["c.name"] == "Acme"

    # --- OPTIONAL MATCH ---

    def test_optional_match_with_results(self, db):
        """OPTIONAL MATCH returns rows when relationship exists."""
        alice = db.create_node(["Person"], {"name": "Alice"})
        bob = db.create_node(["Person"], {"name": "Bob"})
        db.create_edge(alice.id, bob.id, "KNOWS")
        result = list(
            db.execute(
                "MATCH (a:Person {name: 'Alice'}) "
                "OPTIONAL MATCH (a)-[:KNOWS]->(b:Person) "
                "RETURN a.name, b.name"
            )
        )
        assert len(result) >= 1
        assert any(r["b.name"] == "Bob" for r in result)

    def test_optional_match_null(self, db):
        """OPTIONAL MATCH returns null when no relationship exists."""
        db.create_node(["Person"], {"name": "Alice"})
        result = list(
            db.execute(
                "MATCH (a:Person {name: 'Alice'}) "
                "OPTIONAL MATCH (a)-[:MANAGES]->(b) "
                "RETURN a.name, b"
            )
        )
        assert len(result) >= 1

    # --- WHERE ---

    def test_where_comparison(self, db):
        """WHERE clause with comparison operator."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        result = list(db.execute("MATCH (n:Person) WHERE n.age > 28 RETURN n.name"))
        assert len(result) == 1
        assert result[0]["n.name"] == "Alice"

    # --- FOR (GQL equivalent of UNWIND) ---

    def test_for_literal_list(self, db):
        """FOR x IN list, expanding list into rows."""
        result = list(db.execute("FOR x IN [1, 2, 3] RETURN x"))
        values = [r["x"] for r in result]
        assert values == [1, 2, 3]

    def test_for_with_match(self, db):
        """FOR combined with MATCH."""
        db.create_node(["Person"], {"name": "Alice"})
        result = list(
            db.execute(
                "MATCH (n:Person {name: 'Alice'}) FOR x IN [10, 20] RETURN n.name, x"
            )
        )
        assert len(result) == 2

    def test_for_empty_list(self, db):
        """FOR over an empty list produces zero rows."""
        result = list(db.execute("FOR x IN [] RETURN x"))
        assert len(result) == 0

    def test_unwind_literal_list(self, db):
        """UNWIND is also accepted in GQL as an alias for FOR."""
        result = list(db.execute("UNWIND [4, 5, 6] AS x RETURN x"))
        values = [r["x"] for r in result]
        assert values == [4, 5, 6]

    # --- WITH ---

    def test_with_clause(self, db):
        """WITH pipes data between clauses with projection."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        db.create_node(["Person"], {"name": "Charlie", "age": 35})
        result = list(
            db.execute(
                "MATCH (p:Person) "
                "WITH p.name AS name, p.age AS age "
                "WHERE age > 28 "
                "RETURN name ORDER BY name"
            )
        )
        names = [r["name"] for r in result]
        assert names == ["Alice", "Charlie"]

    def test_with_where(self, db):
        """WITH followed by WHERE for filtering."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        result = list(
            db.execute("MATCH (p:Person) WITH p WHERE p.age > 28 RETURN p.name")
        )
        assert len(result) == 1
        assert result[0]["p.name"] == "Alice"

    # --- RETURN / RETURN DISTINCT ---

    def test_return_basic(self, db):
        """Basic RETURN with property access."""
        db.create_node(["Item"], {"val": 42})
        result = list(db.execute("MATCH (n:Item) RETURN n.val"))
        assert result[0]["n.val"] == 42

    def test_return_alias(self, db):
        """RETURN with AS alias."""
        db.create_node(["Item"], {"val": 42})
        result = list(db.execute("MATCH (n:Item) RETURN n.val AS value"))
        assert result[0]["value"] == 42

    def test_return_distinct(self, db):
        """RETURN DISTINCT eliminates duplicate rows."""
        db.create_node(["Person"], {"name": "A", "city": "NYC"})
        db.create_node(["Person"], {"name": "B", "city": "NYC"})
        db.create_node(["Person"], {"name": "C", "city": "LA"})
        result = list(db.execute("MATCH (p:Person) RETURN DISTINCT p.city"))
        cities = {r["p.city"] for r in result}
        assert cities == {"NYC", "LA"}

    # --- ORDER BY / SKIP / LIMIT ---

    def test_order_by_alias_asc(self, db):
        """ORDER BY ascending on alias."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        db.create_node(["Person"], {"name": "Charlie", "age": 35})
        result = list(
            db.execute(
                "MATCH (p:Person) RETURN p.name AS name, p.age AS age ORDER BY age ASC"
            )
        )
        names = [r["name"] for r in result]
        assert names == ["Bob", "Alice", "Charlie"]

    def test_order_by_alias_desc(self, db):
        """ORDER BY descending on alias."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        db.create_node(["Person"], {"name": "Charlie", "age": 35})
        result = list(
            db.execute(
                "MATCH (p:Person) RETURN p.name AS name, p.age AS age ORDER BY age DESC"
            )
        )
        names = [r["name"] for r in result]
        assert names == ["Charlie", "Alice", "Bob"]

    def test_skip(self, db):
        """SKIP skips the first N rows."""
        db.create_node(["Person"], {"name": "A", "age": 1})
        db.create_node(["Person"], {"name": "B", "age": 2})
        db.create_node(["Person"], {"name": "C", "age": 3})
        result = list(
            db.execute(
                "MATCH (p:Person) "
                "RETURN p.name AS name, p.age AS age ORDER BY age SKIP 1"
            )
        )
        assert len(result) == 2

    def test_limit(self, db):
        """LIMIT restricts the number of returned rows."""
        db.create_node(["Person"], {"name": "A"})
        db.create_node(["Person"], {"name": "B"})
        db.create_node(["Person"], {"name": "C"})
        result = list(db.execute("MATCH (p:Person) RETURN p.name LIMIT 2"))
        assert len(result) == 2

    def test_skip_and_limit(self, db):
        """SKIP + LIMIT for pagination."""
        db.create_node(["Person"], {"name": "A", "age": 1})
        db.create_node(["Person"], {"name": "B", "age": 2})
        db.create_node(["Person"], {"name": "C", "age": 3})
        result = list(
            db.execute(
                "MATCH (p:Person) "
                "RETURN p.name AS name, p.age AS age ORDER BY age SKIP 1 LIMIT 1"
            )
        )
        assert len(result) == 1
        assert result[0]["name"] == "B"

    # --- GROUP BY / HAVING ---

    def test_having_filters_aggregates(self, db):
        """HAVING filters groups after aggregation."""
        db.create_node(["Person"], {"name": "A", "city": "NYC"})
        db.create_node(["Person"], {"name": "B", "city": "NYC"})
        db.create_node(["Person"], {"name": "C", "city": "LA"})
        db.create_node(["Person"], {"name": "D", "city": "NYC"})
        result = list(
            db.execute("MATCH (p:Person) RETURN p.city, count(p) AS cnt HAVING cnt > 1")
        )
        assert len(result) == 1
        assert result[0]["p.city"] == "NYC"

    # --- INSERT ---

    def test_insert_node(self, db):
        """INSERT creates a new node."""
        db.execute("INSERT (:Person {name: 'NewNode', age: 42})")
        result = list(db.execute("MATCH (n:Person {name: 'NewNode'}) RETURN n.age"))
        assert len(result) == 1
        assert result[0]["n.age"] == 42

    def test_insert_node_multiple_labels(self, db):
        """INSERT with multiple labels."""
        db.execute("INSERT (:Person:Developer {name: 'MultiLabel'})")
        result = list(db.execute("MATCH (n:Person:Developer) RETURN n.name"))
        assert len(result) >= 1

    def test_insert_edge(self, db):
        """INSERT an edge between existing nodes via MATCH + CREATE."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Person"], {"name": "Bob"})
        db.execute(
            "MATCH (a:Person {name: 'Alice'}), (b:Person {name: 'Bob'}) "
            "CREATE (a)-[:KNOWS {since: 2024}]->(b)"
        )
        result = list(
            db.execute(
                "MATCH (a:Person {name: 'Alice'})-[r:KNOWS]->(b:Person {name: 'Bob'}) "
                "RETURN r.since"
            )
        )
        assert len(result) == 1
        assert result[0]["r.since"] == 2024

    # --- SET property ---

    def test_set_property(self, db):
        """SET updates a property value."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.execute("MATCH (n:Person {name: 'Alice'}) SET n.age = 31")
        result = list(db.execute("MATCH (n:Person {name: 'Alice'}) RETURN n.age"))
        assert result[0]["n.age"] == 31

    def test_set_multiple_properties(self, db):
        """SET with comma-separated assignments."""
        db.create_node(["Person"], {"name": "Alice", "age": 30, "city": "NYC"})
        db.execute("MATCH (n:Person {name: 'Alice'}) SET n.age = 31, n.city = 'LA'")
        result = list(
            db.execute("MATCH (n:Person {name: 'Alice'}) RETURN n.age, n.city")
        )
        assert result[0]["n.age"] == 31
        assert result[0]["n.city"] == "LA"

    def test_set_new_property(self, db):
        """SET adds a property that did not exist before."""
        db.create_node(["Person"], {"name": "Alice"})
        db.execute("MATCH (n:Person {name: 'Alice'}) SET n.email = 'alice@test.com'")
        result = list(db.execute("MATCH (n:Person {name: 'Alice'}) RETURN n.email"))
        assert result[0]["n.email"] == "alice@test.com"

    def test_set_label(self, db):
        """SET n:Label adds a label to an existing node."""
        db.create_node(["Person"], {"name": "Alice"})
        db.execute("MATCH (n:Person {name: 'Alice'}) SET n:Admin")
        result = list(db.execute("MATCH (n:Admin {name: 'Alice'}) RETURN n.name"))
        assert len(result) == 1

    # --- REMOVE ---

    def test_remove_property(self, db):
        """REMOVE n.prop removes a property from a node."""
        db.create_node(["Person"], {"name": "Alice", "temp": "delete_me"})
        db.execute("MATCH (n:Person {name: 'Alice'}) REMOVE n.temp")
        result = list(db.execute("MATCH (n:Person {name: 'Alice'}) RETURN n.temp"))
        assert result[0].get("n.temp") is None

    def test_remove_label(self, db):
        """REMOVE n:Label removes a label from a node."""
        db.create_node(["Person", "Admin"], {"name": "Alice"})
        db.execute("MATCH (n:Admin {name: 'Alice'}) REMOVE n:Admin")
        result = list(db.execute("MATCH (n:Admin {name: 'Alice'}) RETURN n"))
        assert len(result) == 0

    # --- DELETE ---

    def test_delete_node(self, db):
        """DELETE removes a node with no relationships."""
        db.create_node(["Temp"], {"name": "ToDelete"})
        db.execute("MATCH (n:Temp {name: 'ToDelete'}) DELETE n")
        result = list(db.execute("MATCH (n:Temp) RETURN n"))
        assert len(result) == 0

    def test_detach_delete(self, db):
        """DETACH DELETE removes a node and all its relationships."""
        a = db.create_node(["Temp"], {"name": "A"})
        b = db.create_node(["Temp"], {"name": "B"})
        db.create_edge(a.id, b.id, "REL")
        db.execute("MATCH (n:Temp {name: 'A'}) DETACH DELETE n")
        result = list(db.execute("MATCH (n:Temp) RETURN n.name"))
        names = [r["n.name"] for r in result]
        assert "A" not in names
        assert "B" in names

    # --- MERGE ---

    def test_merge_creates_when_absent(self, db):
        """MERGE creates a node when it does not exist."""
        db.execute("MERGE (:City {name: 'Paris'})")
        result = list(db.execute("MATCH (n:City) RETURN count(n) AS cnt"))
        assert result[0]["cnt"] == 1

    def test_merge_matches_when_present(self, db):
        """MERGE matches an existing node, no duplicate."""
        db.execute("INSERT (:City {name: 'Paris'})")
        db.execute("MERGE (:City {name: 'Paris'})")
        result = list(db.execute("MATCH (n:City) RETURN count(n) AS cnt"))
        assert result[0]["cnt"] == 1

    def test_merge_on_create_set(self, db):
        """MERGE ON CREATE SET sets properties only when creating."""
        db.execute(
            "MERGE (n:City {name: 'Berlin'}) ON CREATE SET n.new = true RETURN n"
        )
        result = list(db.execute("MATCH (n:City {name: 'Berlin'}) RETURN n.new"))
        assert result[0]["n.new"] is True

    def test_merge_on_match_set(self, db):
        """MERGE ON MATCH SET sets properties only when matching."""
        db.execute("INSERT (:City {name: 'London'})")
        db.execute(
            "MERGE (n:City {name: 'London'}) ON MATCH SET n.visited = true RETURN n"
        )
        result = list(db.execute("MATCH (n:City {name: 'London'}) RETURN n.visited"))
        assert result[0]["n.visited"] is True

    def test_merge_relationship(self, db):
        """MERGE on a relationship pattern."""
        db.create_node(["Person"], {"name": "X"})
        db.create_node(["Person"], {"name": "Y"})
        db.execute(
            "MATCH (a:Person {name: 'X'}), (b:Person {name: 'Y'}) "
            "MERGE (a)-[r:KNOWS]->(b) RETURN r"
        )
        result = list(
            db.execute(
                "MATCH (a:Person {name: 'X'})-[r:KNOWS]->(b:Person {name: 'Y'}) "
                "RETURN r"
            )
        )
        assert len(result) == 1

    def test_merge_relationship_idempotent(self, db):
        """Running MERGE twice should not create duplicate edges."""
        db.create_node(["Person"], {"name": "M1"})
        db.create_node(["Person"], {"name": "M2"})
        for _ in range(2):
            db.execute(
                "MATCH (a:Person {name: 'M1'}), (b:Person {name: 'M2'}) "
                "MERGE (a)-[r:FRIEND]->(b) RETURN r"
            )
        result = list(
            db.execute(
                "MATCH (:Person {name: 'M1'})-[r:FRIEND]->(:Person {name: 'M2'}) "
                "RETURN r"
            )
        )
        assert len(result) == 1


# =============================================================================
# EXPRESSIONS & OPERATORS
# =============================================================================


class TestGqlExpressions:
    """Test GQL expression types and operators."""

    # --- Arithmetic ---

    def test_addition(self, db):
        """Arithmetic addition in RETURN."""
        db.create_node(["Item"], {"val": 10})
        result = list(db.execute("MATCH (n:Item) RETURN n.val + 5 AS total"))
        assert result[0]["total"] == 15

    def test_subtraction(self, db):
        """Arithmetic subtraction in RETURN."""
        db.create_node(["Item"], {"val": 10})
        result = list(db.execute("MATCH (n:Item) RETURN n.val - 3 AS diff"))
        assert result[0]["diff"] == 7

    def test_multiplication(self, db):
        """Arithmetic multiplication."""
        db.create_node(["Item"], {"val": 6})
        result = list(db.execute("MATCH (n:Item) RETURN n.val * 7 AS product"))
        assert result[0]["product"] == 42

    def test_division(self, db):
        """Arithmetic division."""
        db.create_node(["Item"], {"val": 10})
        result = list(db.execute("MATCH (n:Item) RETURN n.val / 2 AS half"))
        assert result[0]["half"] == 5

    # --- Comparison ---

    def test_equals(self, db):
        """Equality comparison in WHERE."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Person"], {"name": "Bob"})
        result = list(
            db.execute("MATCH (n:Person) WHERE n.name = 'Alice' RETURN n.name")
        )
        assert len(result) == 1

    def test_not_equals(self, db):
        """Inequality comparison (<>)."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Person"], {"name": "Bob"})
        result = list(
            db.execute("MATCH (n:Person) WHERE n.name <> 'Alice' RETURN n.name")
        )
        assert len(result) == 1
        assert result[0]["n.name"] == "Bob"

    def test_less_than(self, db):
        """Less-than comparison."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        result = list(db.execute("MATCH (n:Person) WHERE n.age < 28 RETURN n.name"))
        assert len(result) == 1
        assert result[0]["n.name"] == "Bob"

    def test_greater_equal(self, db):
        """Greater-or-equal comparison."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        result = list(db.execute("MATCH (n:Person) WHERE n.age >= 30 RETURN n.name"))
        assert len(result) == 1
        assert result[0]["n.name"] == "Alice"

    # --- Logical ---

    def test_and(self, db):
        """Logical AND in WHERE."""
        db.create_node(["Person"], {"name": "Alice", "age": 30, "city": "NYC"})
        db.create_node(["Person"], {"name": "Bob", "age": 25, "city": "LA"})
        db.create_node(["Person"], {"name": "Charlie", "age": 35, "city": "NYC"})
        result = list(
            db.execute(
                "MATCH (n:Person) WHERE n.age > 25 AND n.city = 'NYC' RETURN n.name"
            )
        )
        names = {r["n.name"] for r in result}
        assert names == {"Alice", "Charlie"}

    def test_or(self, db):
        """Logical OR in WHERE."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Bob", "age": 25})
        db.create_node(["Person"], {"name": "Charlie", "age": 35})
        result = list(
            db.execute(
                "MATCH (n:Person) WHERE n.name = 'Alice' OR n.name = 'Bob' "
                "RETURN n.name"
            )
        )
        assert len(result) == 2

    def test_not(self, db):
        """Logical NOT in WHERE."""
        db.create_node(["Person"], {"name": "Alice", "city": "NYC"})
        db.create_node(["Person"], {"name": "Bob", "city": "LA"})
        result = list(
            db.execute("MATCH (n:Person) WHERE NOT n.city = 'NYC' RETURN n.name")
        )
        assert len(result) == 1
        assert result[0]["n.name"] == "Bob"

    # --- CASE ---

    def test_case_simple(self, db):
        """Simple CASE expression (CASE expr WHEN val THEN result)."""
        db.create_node(["Person"], {"name": "Alice", "city": "NYC"})
        result = list(
            db.execute(
                "MATCH (n:Person {name: 'Alice'}) "
                "RETURN CASE n.city WHEN 'NYC' THEN 'East' WHEN 'LA' THEN 'West' END AS coast"
            )
        )
        assert result[0]["coast"] == "East"

    def test_case_searched(self, db):
        """Searched CASE expression (CASE WHEN cond THEN result)."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        db.create_node(["Person"], {"name": "Charlie", "age": 35})
        result = list(
            db.execute(
                "MATCH (n:Person {name: 'Charlie'}) "
                "RETURN CASE WHEN n.age > 30 THEN 'senior' ELSE 'junior' END AS level"
            )
        )
        assert result[0]["level"] == "senior"

    # --- Literals ---

    def test_list_literal(self, db):
        """List literal expression [1, 2, 3]."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN [1, 2, 3] AS nums"))
        assert result[0]["nums"] == [1, 2, 3]

    def test_map_literal(self, db):
        """Map literal expression {a: 1, b: 2}."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN {a: 1, b: 2} AS m"))
        assert result[0]["m"]["a"] == 1
        assert result[0]["m"]["b"] == 2

    # --- Binary expression in RETURN ---

    def test_binary_expr_in_return(self, db):
        """Binary comparison in RETURN: count(n) > 0 AS has."""
        db.create_node(["Person"], {"name": "Alice"})
        result = list(db.execute("MATCH (n:Person) RETURN count(n) > 0 AS has_people"))
        assert result[0]["has_people"] is True

    def test_arithmetic_in_return(self, db):
        """Arithmetic in RETURN: n.age + 10."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        result = list(
            db.execute(
                "MATCH (n:Person {name: 'Alice'}) RETURN n.age + 10 AS future_age"
            )
        )
        assert result[0]["future_age"] == 40

    def test_comparison_in_return(self, db):
        """Comparison in RETURN: n.age > 25."""
        db.create_node(["Person"], {"name": "Alice", "age": 30})
        result = list(
            db.execute(
                "MATCH (n:Person {name: 'Alice'}) RETURN n.age > 25 AS is_over_25"
            )
        )
        assert result[0]["is_over_25"] is True

    # --- Parameters ---

    def test_parameter_in_where(self, db):
        """Parameterized query with $name."""
        db.create_node(["Person"], {"name": "ParamTest", "age": 42})
        result = list(
            db.execute(
                "MATCH (n:Person) WHERE n.name = $name RETURN n.age",
                {"name": "ParamTest"},
            )
        )
        assert len(result) == 1
        assert result[0]["n.age"] == 42

    # --- String predicates ---

    def test_starts_with(self, db):
        """STARTS WITH string predicate."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Person"], {"name": "Bob"})
        result = list(
            db.execute("MATCH (n:Person) WHERE n.name STARTS WITH 'A' RETURN n.name")
        )
        assert len(result) == 1
        assert result[0]["n.name"] == "Alice"

    def test_ends_with(self, db):
        """ENDS WITH string predicate."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Person"], {"name": "Charlie"})
        db.create_node(["Person"], {"name": "Bob"})
        result = list(
            db.execute("MATCH (n:Person) WHERE n.name ENDS WITH 'e' RETURN n.name")
        )
        names = {r["n.name"] for r in result}
        assert "Alice" in names
        assert "Charlie" in names

    def test_contains(self, db):
        """CONTAINS string predicate."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Person"], {"name": "Bob"})
        result = list(
            db.execute("MATCH (n:Person) WHERE n.name CONTAINS 'lic' RETURN n.name")
        )
        assert len(result) == 1
        assert result[0]["n.name"] == "Alice"

    # --- IN operator ---

    def test_in_list(self, db):
        """IN list membership test."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Person"], {"name": "Bob"})
        db.create_node(["Person"], {"name": "Charlie"})
        result = list(
            db.execute(
                "MATCH (n:Person) WHERE n.name IN ['Alice', 'Bob'] RETURN n.name"
            )
        )
        assert len(result) == 2


# =============================================================================
# PREDICATES
# =============================================================================


class TestGqlPredicates:
    """Test GQL predicates (EXISTS, IS NULL, IS NOT NULL)."""

    def test_is_null(self, db):
        """IS NULL predicate for missing property."""
        db.create_node(["Item"], {"name": "WithProp", "val": 1})
        db.create_node(["Item"], {"name": "NoProp"})
        result = list(db.execute("MATCH (n:Item) WHERE n.val IS NULL RETURN n.name"))
        assert len(result) == 1
        assert result[0]["n.name"] == "NoProp"

    def test_is_not_null(self, db):
        """IS NOT NULL predicate for existing property."""
        db.create_node(["Item"], {"name": "WithProp", "val": 1})
        db.create_node(["Item"], {"name": "NoProp"})
        result = list(
            db.execute("MATCH (n:Item) WHERE n.val IS NOT NULL RETURN n.name")
        )
        assert len(result) == 1
        assert result[0]["n.name"] == "WithProp"

    def test_exists_subquery(self, db):
        """EXISTS { MATCH ... } subquery predicate."""
        alice = db.create_node(["Person"], {"name": "Alice"})
        bob = db.create_node(["Person"], {"name": "Bob"})
        charlie = db.create_node(["Person"], {"name": "Charlie"})
        db.create_edge(alice.id, bob.id, "KNOWS")
        db.create_edge(alice.id, charlie.id, "KNOWS")
        result = list(
            db.execute(
                "MATCH (p:Person) WHERE EXISTS { MATCH (p)-[:KNOWS]->() } RETURN p.name"
            )
        )
        names = {r["p.name"] for r in result}
        assert "Alice" in names

    def test_null_property_access(self, db):
        """Accessing a nonexistent property returns null."""
        db.create_node(["Person"], {"name": "Alice"})
        result = list(
            db.execute("MATCH (n:Person {name: 'Alice'}) RETURN n.nonexistent")
        )
        assert len(result) == 1


# =============================================================================
# PATTERN FEATURES
# =============================================================================


class TestGqlPatterns:
    """Test GQL pattern matching capabilities."""

    def test_node_pattern(self, db):
        """Basic node pattern (n)."""
        db.create_node(["A"], {"v": 1})
        db.create_node(["B"], {"v": 2})
        result = list(db.execute("MATCH (n) RETURN count(n) AS cnt"))
        assert result[0]["cnt"] == 2

    def test_directed_edge_pattern(self, db):
        """Directed edge pattern (a)-[:TYPE]->(b)."""
        a = db.create_node(["Person"], {"name": "Alice"})
        b = db.create_node(["Person"], {"name": "Bob"})
        db.create_edge(a.id, b.id, "KNOWS")
        result = list(
            db.execute("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name")
        )
        assert len(result) == 1
        assert result[0]["a.name"] == "Alice"
        assert result[0]["b.name"] == "Bob"

    def test_undirected_edge_pattern(self, db):
        """Undirected edge pattern (a)-[:TYPE]-(b)."""
        a = db.create_node(["Person"], {"name": "Alice"})
        b = db.create_node(["Person"], {"name": "Bob"})
        db.create_edge(a.id, b.id, "KNOWS")
        result = list(
            db.execute(
                "MATCH (a:Person {name: 'Alice'})-[:KNOWS]-(b:Person) RETURN b.name"
            )
        )
        names = {r["b.name"] for r in result}
        assert "Bob" in names

    def test_edge_with_properties(self, db):
        """Edge pattern with property filter."""
        a = db.create_node(["Person"], {"name": "Alice"})
        b = db.create_node(["Person"], {"name": "Bob"})
        db.create_edge(a.id, b.id, "KNOWS", {"since": 2020})
        result = list(
            db.execute(
                "MATCH (a:Person)-[r:KNOWS]->(b:Person) "
                "WHERE r.since = 2020 "
                "RETURN a.name, b.name"
            )
        )
        assert len(result) == 1

    def test_multi_hop_pattern(self, db):
        """Two-hop path pattern."""
        a = db.create_node(["Person"], {"name": "Alice"})
        b = db.create_node(["Person"], {"name": "Bob"})
        c = db.create_node(["Person"], {"name": "Charlie"})
        db.create_edge(a.id, b.id, "KNOWS")
        db.create_edge(b.id, c.id, "KNOWS")
        result = list(
            db.execute(
                "MATCH (a:Person)-[:KNOWS]->(b:Person)-[:KNOWS]->(c:Person) "
                "RETURN a.name, c.name"
            )
        )
        assert len(result) == 1
        assert result[0]["a.name"] == "Alice"
        assert result[0]["c.name"] == "Charlie"

    def test_comma_separated_patterns(self, db):
        """Comma-separated patterns in MATCH (cross product)."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Company"], {"name": "Acme"})
        result = list(
            db.execute(
                "MATCH (a:Person {name: 'Alice'}), (b:Company {name: 'Acme'}) "
                "RETURN a.name, b.name"
            )
        )
        assert len(result) == 1
        assert result[0]["a.name"] == "Alice"
        assert result[0]["b.name"] == "Acme"

    def test_variable_length_path(self, db):
        """Variable-length path: -[:TYPE*min..max]->."""
        a = db.create_node(["Node"], {"name": "a"})
        b = db.create_node(["Node"], {"name": "b"})
        c = db.create_node(["Node"], {"name": "c"})
        d = db.create_node(["Node"], {"name": "d"})
        db.create_edge(a.id, b.id, "NEXT")
        db.create_edge(b.id, c.id, "NEXT")
        db.create_edge(c.id, d.id, "NEXT")
        result = list(
            db.execute(
                "MATCH (start:Node {name: 'a'})-[:NEXT*1..3]->(end_node:Node) "
                "RETURN end_node.name"
            )
        )
        names = {r["end_node.name"] for r in result}
        assert "b" in names
        assert "c" in names
        assert "d" in names

    def test_named_path(self, db):
        """Named path: p = (a)-[]->(b)."""
        a = db.create_node(["Node"], {"name": "a"})
        b = db.create_node(["Node"], {"name": "b"})
        db.create_edge(a.id, b.id, "NEXT")
        result = list(
            db.execute(
                "MATCH p = (start:Node {name: 'a'})-[:NEXT]->(dest:Node) "
                "RETURN length(p) AS path_len"
            )
        )
        assert result[0]["path_len"] == 1

    def test_shortest_path(self, db):
        """shortestPath function."""
        a = db.create_node(["Node"], {"name": "a"})
        b = db.create_node(["Node"], {"name": "b"})
        c = db.create_node(["Node"], {"name": "c"})
        d = db.create_node(["Node"], {"name": "d"})
        db.create_edge(a.id, b.id, "STEP")
        db.create_edge(b.id, c.id, "STEP")
        db.create_edge(c.id, d.id, "STEP")
        db.create_edge(a.id, d.id, "DIRECT")
        result = list(
            db.execute(
                "MATCH p = shortestPath("
                "(start:Node {name: 'a'})-[*]-(dest:Node {name: 'd'})"
                ") RETURN length(p) AS path_len"
            )
        )
        assert result[0]["path_len"] == 1

    def test_all_shortest_paths(self, db):
        """allShortestPaths function."""
        a = db.create_node(["Node"], {"name": "a"})
        b = db.create_node(["Node"], {"name": "b"})
        c = db.create_node(["Node"], {"name": "c"})
        d = db.create_node(["Node"], {"name": "d"})
        db.create_edge(a.id, b.id, "EDGE")
        db.create_edge(a.id, c.id, "EDGE")
        db.create_edge(b.id, d.id, "EDGE")
        db.create_edge(c.id, d.id, "EDGE")
        result = list(
            db.execute(
                "MATCH p = allShortestPaths("
                "(a:Node {name: 'a'})-[*]-(d:Node {name: 'd'})"
                ") RETURN length(p) AS len"
            )
        )
        assert len(result) >= 2
        assert all(r["len"] == 2 for r in result)

    def test_no_path_returns_empty(self, db):
        """Non-existent path returns zero rows."""
        db.create_node(["Node"], {"name": "a"})
        db.create_node(["Node"], {"name": "b"})
        result = list(
            db.execute(
                "MATCH (a:Node {name: 'a'})-[:EDGE]->(b:Node {name: 'b'}) RETURN a, b"
            )
        )
        assert len(result) == 0


# =============================================================================
# BUILT-IN FUNCTIONS
# =============================================================================


class TestGqlFunctions:
    """Test GQL built-in scalar and aggregate functions."""

    # --- Aggregate functions ---

    def test_count(self, db):
        """count(n) aggregate."""
        db.create_node(["Person"], {"name": "A"})
        db.create_node(["Person"], {"name": "B"})
        db.create_node(["Person"], {"name": "C"})
        result = list(db.execute("MATCH (n:Person) RETURN count(n) AS cnt"))
        assert result[0]["cnt"] == 3

    def test_count_distinct(self, db):
        """count(DISTINCT expr) aggregate."""
        db.create_node(["Person"], {"name": "A", "city": "NYC"})
        db.create_node(["Person"], {"name": "B", "city": "NYC"})
        db.create_node(["Person"], {"name": "C", "city": "LA"})
        result = list(
            db.execute("MATCH (n:Person) RETURN count(DISTINCT n.city) AS cnt")
        )
        assert result[0]["cnt"] == 2

    def test_sum(self, db):
        """sum(expr) aggregate."""
        db.create_node(["Person"], {"name": "A", "age": 30})
        db.create_node(["Person"], {"name": "B", "age": 25})
        db.create_node(["Person"], {"name": "C", "age": 35})
        result = list(db.execute("MATCH (n:Person) RETURN sum(n.age) AS total"))
        assert result[0]["total"] == 90

    def test_avg(self, db):
        """avg(expr) aggregate."""
        db.create_node(["Person"], {"name": "A", "age": 30})
        db.create_node(["Person"], {"name": "B", "age": 25})
        db.create_node(["Person"], {"name": "C", "age": 35})
        result = list(db.execute("MATCH (n:Person) RETURN avg(n.age) AS average"))
        assert result[0]["average"] == 30.0

    def test_min(self, db):
        """min(expr) aggregate."""
        db.create_node(["Person"], {"name": "A", "age": 30})
        db.create_node(["Person"], {"name": "B", "age": 25})
        db.create_node(["Person"], {"name": "C", "age": 35})
        result = list(db.execute("MATCH (n:Person) RETURN min(n.age) AS youngest"))
        assert result[0]["youngest"] == 25

    def test_max(self, db):
        """max(expr) aggregate."""
        db.create_node(["Person"], {"name": "A", "age": 30})
        db.create_node(["Person"], {"name": "B", "age": 25})
        db.create_node(["Person"], {"name": "C", "age": 35})
        result = list(db.execute("MATCH (n:Person) RETURN max(n.age) AS oldest"))
        assert result[0]["oldest"] == 35

    def test_collect(self, db):
        """collect(expr) aggregate."""
        db.create_node(["Person"], {"name": "Alice"})
        db.create_node(["Person"], {"name": "Bob"})
        db.create_node(["Person"], {"name": "Charlie"})
        result = list(db.execute("MATCH (p:Person) RETURN collect(p.name) AS names"))
        assert len(result) == 1
        names = result[0]["names"]
        assert set(names) == {"Alice", "Bob", "Charlie"}

    def test_stdev(self, db):
        """stdev(expr) sample standard deviation."""
        for v in [10, 20, 30]:
            db.create_node(["Score"], {"val": v})
        result = list(db.execute("MATCH (n:Score) RETURN stdev(n.val) AS sd"))
        assert 8 <= result[0]["sd"] <= 12

    # --- Scalar functions ---

    def test_labels(self, db):
        """labels(n) returns the labels of a node."""
        db.create_node(["Person", "Employee"], {"name": "Alice"})
        result = list(db.execute("MATCH (n {name: 'Alice'}) RETURN labels(n) AS lbls"))
        lbls = result[0]["lbls"]
        assert "Person" in lbls
        assert "Employee" in lbls

    def test_size_list(self, db):
        """size(list) returns list length."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN size([1, 2, 3]) AS s"))
        assert result[0]["s"] == 3

    def test_size_string(self, db):
        """size(string) returns string length."""
        db.create_node(["X"], {"name": "hello"})
        result = list(db.execute("MATCH (n:X) RETURN size(n.name) AS s"))
        assert result[0]["s"] == 5

    def test_coalesce(self, db):
        """coalesce(a, b, ...) returns first non-null."""
        db.create_node(["Item"], {"name": "Test"})
        result = list(
            db.execute(
                "MATCH (n:Item {name: 'Test'}) "
                "RETURN coalesce(n.missing, 'default') AS val"
            )
        )
        assert result[0]["val"] == "default"

    def test_type_function(self, db):
        """type(r) returns the relationship type."""
        a = db.create_node(["Person"], {"name": "Alice"})
        b = db.create_node(["Person"], {"name": "Bob"})
        db.create_edge(a.id, b.id, "KNOWS")
        result = list(
            db.execute(
                "MATCH (a:Person {name: 'Alice'})-[r]->(b:Person) "
                "RETURN type(r) AS rel_type"
            )
        )
        assert result[0]["rel_type"] == "KNOWS"

    def test_id_function(self, db):
        """id(n) returns the internal node identifier."""
        db.create_node(["Person"], {"name": "Alice"})
        result = list(
            db.execute("MATCH (n:Person {name: 'Alice'}) RETURN id(n) AS nid")
        )
        assert result[0]["nid"] is not None

    def test_head(self, db):
        """head(list) returns first element."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN head([1, 2, 3]) AS h"))
        assert result[0]["h"] == 1

    def test_tail(self, db):
        """tail(list) returns all elements except first."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN tail([1, 2, 3]) AS t"))
        assert result[0]["t"] == [2, 3]

    def test_last(self, db):
        """last(list) returns the last element."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN last([1, 2, 3]) AS l"))
        assert result[0]["l"] == 3

    def test_reverse_list(self, db):
        """reverse(list) reverses a list."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN reverse([1, 2, 3]) AS val"))
        assert result[0]["val"] == [3, 2, 1]

    def test_to_integer(self, db):
        """toInteger(string) converts a string to integer."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN toInteger('42') AS val"))
        assert result[0]["val"] == 42

    def test_to_float(self, db):
        """toFloat(string) converts a string to float."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN toFloat('3.14') AS val"))
        assert abs(result[0]["val"] - 3.14) < 0.001

    def test_to_string(self, db):
        """toString(expr) converts a value to string."""
        db.create_node(["X"], {"v": 42})
        result = list(db.execute("MATCH (n:X) RETURN toString(n.v) AS val"))
        assert "42" in str(result[0]["val"])

    def test_to_boolean(self, db):
        """toBoolean(string) converts a string to boolean."""
        db.create_node(["X"], {"v": 1})
        result = list(db.execute("MATCH (n:X) RETURN toBoolean('true') AS val"))
        assert result[0]["val"] is True

    def test_exists_property(self, db):
        """exists(n.prop) checks whether a property exists."""
        db.create_node(["Item"], {"name": "With", "email": "a@b.com"})
        db.create_node(["Item"], {"name": "Without"})
        result = list(db.execute("MATCH (n:Item) WHERE exists(n.email) RETURN n.name"))
        assert len(result) == 1
        assert result[0]["n.name"] == "With"

    # --- Path functions ---

    def test_length_function(self, db):
        """length(path) returns edge count in a path."""
        a = db.create_node(["Node"], {"name": "a"})
        b = db.create_node(["Node"], {"name": "b"})
        c = db.create_node(["Node"], {"name": "c"})
        db.create_edge(a.id, b.id, "NEXT")
        db.create_edge(b.id, c.id, "NEXT")
        result = list(
            db.execute(
                "MATCH p = (start:Node {name: 'a'})-[:NEXT*]->(dest:Node {name: 'c'}) "
                "RETURN length(p) AS len"
            )
        )
        assert result[0]["len"] == 2
