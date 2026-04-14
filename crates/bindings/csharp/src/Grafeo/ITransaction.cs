namespace Grafeo;

/// <summary>
/// Interface for a Grafeo transaction, enabling testability via mocking.
/// </summary>
public interface ITransaction : IDisposable, IAsyncDisposable
{
    QueryResult Execute(string query);
    Task<QueryResult> ExecuteAsync(string query, CancellationToken ct = default);
    QueryResult ExecuteWithParams(string query, Dictionary<string, object?> parameters);
    QueryResult ExecuteLanguage(string language, string query, string? paramsJson = null);
    void Commit();
    void Rollback();
}
