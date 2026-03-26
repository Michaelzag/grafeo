using Xunit;

namespace Grafeo.Tests;

/// <summary>Aggregate, GROUP BY, and ORDER BY query tests (covers issue #187 end-to-end).</summary>
public sealed class AggregateQueryTests
{
    [Fact]
    public void CountWithGroupBy()
    {
        using var db = GrafeoDB.Memory();
        db.Execute("INSERT (:Person {name: 'Alix'})");
        db.Execute("INSERT (:Person {name: 'Gus'})");
        db.Execute("INSERT (:City {name: 'Amsterdam'})");

        var result = db.Execute(
            "MATCH (n) RETURN labels(n)[0] AS label, count(n) AS cnt ORDER BY label");

        Assert.Equal(2, result.Rows.Count);
        Assert.Equal("City", result.Rows[0]["label"]);
        Assert.Equal(1L, result.Rows[0]["cnt"]);
        Assert.Equal("Person", result.Rows[1]["label"]);
        Assert.Equal(2L, result.Rows[1]["cnt"]);
    }

    [Fact]
    public void SumAggregate()
    {
        using var db = GrafeoDB.Memory();
        db.Execute("INSERT (:Person {name: 'Vincent', age: 40})");
        db.Execute("INSERT (:Person {name: 'Jules', age: 35})");
        db.Execute("INSERT (:Person {name: 'Mia', age: 25})");

        var result = db.Execute("MATCH (p:Person) RETURN sum(p.age) AS total");

        Assert.Single(result.Rows);
        Assert.Equal(100L, result.Rows[0]["total"]);
    }

    [Fact]
    public void OrderByDescending()
    {
        using var db = GrafeoDB.Memory();
        db.Execute("INSERT (:Person {name: 'Butch', age: 50})");
        db.Execute("INSERT (:Person {name: 'Django', age: 30})");
        db.Execute("INSERT (:Person {name: 'Shosanna', age: 40})");

        var result = db.Execute(
            "MATCH (p:Person) RETURN p.name, p.age ORDER BY p.age DESC");

        Assert.Equal(3, result.Rows.Count);
        Assert.Equal("Butch", result.Rows[0]["p.name"]);
        Assert.Equal("Shosanna", result.Rows[1]["p.name"]);
        Assert.Equal("Django", result.Rows[2]["p.name"]);
    }

    [Fact]
    public void LabelsInGroupBy()
    {
        using var db = GrafeoDB.Memory();
        db.Execute("INSERT (:Person {name: 'Hans'})");
        db.Execute("INSERT (:Person {name: 'Beatrix'})");
        db.Execute("INSERT (:City {name: 'Berlin'})");
        db.Execute("INSERT (:City {name: 'Paris'})");
        db.Execute("INSERT (:City {name: 'Prague'})");

        var result = db.Execute(
            "MATCH (n) RETURN labels(n)[0] AS label, count(n) AS cnt ORDER BY label");

        Assert.Equal(2, result.Rows.Count);
        Assert.Equal("City", result.Rows[0]["label"]);
        Assert.Equal(3L, result.Rows[0]["cnt"]);
        Assert.Equal("Person", result.Rows[1]["label"]);
        Assert.Equal(2L, result.Rows[1]["cnt"]);
    }

    [Fact]
    public void TypeInGroupBy()
    {
        using var db = GrafeoDB.Memory();
        db.Execute("INSERT (:Person {name: 'Alix'})-[:KNOWS]->(:Person {name: 'Gus'})");
        db.Execute("INSERT (:Person {name: 'Vincent'})-[:WORKS_WITH]->(:Person {name: 'Jules'})");
        db.Execute("INSERT (:Person {name: 'Mia'})-[:KNOWS]->(:Person {name: 'Butch'})");

        var result = db.Execute(
            "MATCH ()-[r]->() RETURN type(r) AS rel_type, count(r) AS cnt ORDER BY rel_type");

        Assert.Equal(2, result.Rows.Count);
        Assert.Equal("KNOWS", result.Rows[0]["rel_type"]);
        Assert.Equal(2L, result.Rows[0]["cnt"]);
        Assert.Equal("WORKS_WITH", result.Rows[1]["rel_type"]);
        Assert.Equal(1L, result.Rows[1]["cnt"]);
    }
}
