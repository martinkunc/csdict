using System.Runtime.InteropServices;

namespace CSGtk.App.Sqlite;

/// <summary>Thin ergonomic wrapper over Sqlite3's raw P/Invoke surface: open a read-only
/// connection, run a parameterized query, get mapped rows back.</summary>
internal sealed class SqliteDatabase : IDisposable
{
    private readonly nint _db;

    private SqliteDatabase(nint db) => _db = db;

    public static SqliteDatabase? TryOpen(string path)
    {
        int rc = Sqlite3.sqlite3_open_v2(path, out nint db, Sqlite3.SQLITE_OPEN_READONLY, 0);
        if (rc != Sqlite3.SQLITE_OK)
        {
            if (db != 0)
            {
                Sqlite3.sqlite3_close_v2(db);
            }

            return null;
        }

        return new SqliteDatabase(db);
    }

    public List<T> Query<T>(string sql, IReadOnlyList<string>? parameters, Func<SqliteRow, T> map)
    {
        int rc = Sqlite3.sqlite3_prepare_v2(_db, sql, -1, out nint stmt, 0);
        if (rc != Sqlite3.SQLITE_OK)
        {
            throw new InvalidOperationException($"sqlite3_prepare_v2 failed ({rc}): {GetErrorMessage()}");
        }

        try
        {
            if (parameters is not null)
            {
                for (int i = 0; i < parameters.Count; i++)
                {
                    Sqlite3.sqlite3_bind_text(stmt, i + 1, parameters[i], -1, Sqlite3.SQLITE_TRANSIENT);
                }
            }

            var results = new List<T>();
            var row = new SqliteRow(stmt);
            while (Sqlite3.sqlite3_step(stmt) == Sqlite3.SQLITE_ROW)
            {
                results.Add(map(row));
            }

            return results;
        }
        finally
        {
            Sqlite3.sqlite3_finalize(stmt);
        }
    }

    private string GetErrorMessage() => Marshal.PtrToStringUTF8(Sqlite3.sqlite3_errmsg(_db)) ?? "unknown error";

    public void Dispose() => Sqlite3.sqlite3_close_v2(_db);
}

/// <summary>A single result row, valid only for the duration of the map callback that receives it
/// (columns read straight off the live statement, text copied out immediately).</summary>
internal readonly struct SqliteRow(nint stmt)
{
    public string? GetString(int col)
    {
        if (Sqlite3.sqlite3_column_type(stmt, col) == Sqlite3.SQLITE_NULL)
        {
            return null;
        }

        return Marshal.PtrToStringUTF8(Sqlite3.sqlite3_column_text(stmt, col));
    }
}
