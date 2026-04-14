using Xunit;

namespace Grafeo.Tests;

/// <summary>Tests for schema management, plan cache, projections, and CDC APIs.</summary>
public sealed class SchemaAndAdminTests : IDisposable
{
    private readonly GrafeoDB _db = GrafeoDB.Memory();

    public void Dispose() => _db.Dispose();

    [Fact]
    public void SchemaRoundtrip()
    {
        Assert.Null(_db.CurrentSchema());

        _db.SetSchema("test_schema");
        Assert.Equal("test_schema", _db.CurrentSchema());

        _db.ResetSchema();
        Assert.Null(_db.CurrentSchema());
    }

    [Fact]
    public void ClearPlanCacheDoesNotThrow()
    {
        _db.Execute("INSERT (:A {v: 1})");
        _db.ClearPlanCache();
        // Verify queries still work after cache clear
        var result = _db.Execute("MATCH (a:A) RETURN a.v");
        Assert.Single(result.Rows);
    }

    [Fact]
    public void ProjectionLifecycle()
    {
        _db.Execute("INSERT (:City {name: 'Amsterdam'})");
        _db.Execute("INSERT (:City {name: 'Berlin'})");

        // Create with empty filters (projects all nodes/edges): no string array marshalling
        var created = _db.CreateProjection("all_data");
        Assert.True(created);

        var list = _db.ListProjections();
        Assert.Contains("all_data", list);

        var dropped = _db.DropProjection("all_data");
        Assert.True(dropped);

        var droppedAgain = _db.DropProjection("all_data");
        Assert.False(droppedAgain);
    }

    [Fact]
    public void ProjectionWithFilters()
    {
        _db.Execute("INSERT (:City {name: 'Amsterdam'})");
        _db.Execute("INSERT (:Person {name: 'Alix'})");

        // Create with label filter: exercises string array marshalling.
        // If the native call fails (marshalling issue on some platforms),
        // skip rather than crash CI.
        bool created;
        try
        {
            created = _db.CreateProjection("cities", nodeLabels: ["City"]);
        }
        catch (Exception ex)
        {
            // Skip: platform-specific marshalling issue
            Assert.Fail($"CreateProjection with filters not supported on this platform: {ex.Message}");
            return;
        }

        Assert.True(created);

        var list = _db.ListProjections();
        Assert.Contains("cities", list);

        _db.DropProjection("cities");
    }

    [Fact]
    public void DropNonexistentProjection()
    {
        var dropped = _db.DropProjection("nonexistent");
        Assert.False(dropped);
    }

    [Fact]
    public void InterfaceAssignment()
    {
        // Verify GrafeoDB implements IGrafeoDB
        IGrafeoDB db = _db;
        var result = db.Execute("RETURN 42 AS answer");
        Assert.Single(result.Rows);
    }

    [Fact]
    public void TransactionInterface()
    {
        // Verify Transaction implements ITransaction
        using ITransaction tx = _db.BeginTransaction();
        tx.Execute("INSERT (:Test {v: 1})");
        tx.Commit();
    }
}
