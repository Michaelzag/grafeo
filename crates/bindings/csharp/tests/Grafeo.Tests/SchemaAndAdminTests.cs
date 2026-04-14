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

        // Create the schema first (schemas must exist before SetSchema)
        _db.Execute("CREATE SCHEMA test_schema");

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
    public void ProjectionListAndDrop()
    {
        // ListProjections should return valid JSON even with no projections
        var list = _db.ListProjections();
        Assert.NotNull(list);

        // DropProjection on non-existent should return false
        Assert.False(_db.DropProjection("nonexistent"));
    }

    // NOTE: CreateProjection passes string arrays (const char**) through FFI.
    // The current P/Invoke marshalling crashes the native host on CI runners
    // (both empty-filter and filtered paths). A JSON-based C API variant
    // (grafeo_create_projection_json) is needed for safe cross-platform use.
    // The Rust-side C FFI tests cover this function directly.

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
        // Verify Transaction implements ITransaction via interface
        IGrafeoDB db = _db;
        using var tx = db.BeginTransaction();
        tx.Execute("INSERT (:Test {v: 1})");
        tx.Commit();
    }
}
