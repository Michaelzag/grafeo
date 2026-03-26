using Xunit;

namespace Grafeo.Tests;

/// <summary>Vector index management tests (DropVectorIndex return value correctness).</summary>
public sealed class VectorTests
{
    [Fact]
    public void DropVectorIndexReturnsTrueOnSuccess()
    {
        using var db = GrafeoDB.Memory();

        // Create a node and a vector index via GQL
        db.Execute("INSERT (:Doc {title: 'test', emb: [1.0, 2.0, 3.0]})");
        db.Execute("CREATE VECTOR INDEX doc_emb ON :Doc(emb) DIMENSION 3");

        var dropped = db.DropVectorIndex("Doc", "emb");
        Assert.True(dropped);
    }

    [Fact]
    public void DropNonExistentVectorIndexReturnsFalse()
    {
        using var db = GrafeoDB.Memory();

        var dropped = db.DropVectorIndex("NonExistent", "embedding");
        Assert.False(dropped);
    }
}
