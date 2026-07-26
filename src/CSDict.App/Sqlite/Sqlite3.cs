using System.Runtime.InteropServices;

namespace CSDict.App.Sqlite;

/// <summary>P/Invoke bindings for the subset of libsqlite3 needed for read-only querying:
/// open a connection, prepare/bind/step/finalize a statement, read column values.</summary>
internal static partial class Sqlite3
{
    private const string LibName = "sqlite3";

    public const int SQLITE_OK = 0;
    public const int SQLITE_ROW = 100;
    public const int SQLITE_DONE = 101;
    public const int SQLITE_NULL = 5;
    public const int SQLITE_OPEN_READONLY = 0x00000001;
    public const int SQLITE_OPEN_READWRITE = 0x00000002;
    public const int SQLITE_OPEN_CREATE = 0x00000004;

    /// <summary>SQLITE_TRANSIENT: tells sqlite3_bind_text to copy the string immediately, so the
    /// buffer marshalled in for the call can be freed as soon as the call returns.</summary>
    public static readonly nint SQLITE_TRANSIENT = new(-1);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial int sqlite3_open_v2(string filename, out nint db, int flags, nint vfs);

    [LibraryImport(LibName)]
    public static partial int sqlite3_close_v2(nint db);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial int sqlite3_prepare_v2(nint db, string sql, int nByte, out nint stmt, nint tail);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial int sqlite3_bind_text(nint stmt, int index, string val, int nByte, nint destructor);

    [LibraryImport(LibName)]
    public static partial int sqlite3_bind_null(nint stmt, int index);

    [LibraryImport(LibName)]
    public static partial int sqlite3_bind_int64(nint stmt, int index, long val);

    [LibraryImport(LibName)]
    public static partial int sqlite3_step(nint stmt);

    [LibraryImport(LibName)]
    public static partial nint sqlite3_column_text(nint stmt, int col);

    [LibraryImport(LibName)]
    public static partial int sqlite3_column_type(nint stmt, int col);

    [LibraryImport(LibName)]
    public static partial int sqlite3_reset(nint stmt);

    [LibraryImport(LibName)]
    public static partial int sqlite3_clear_bindings(nint stmt);

    [LibraryImport(LibName)]
    public static partial int sqlite3_finalize(nint stmt);

    [LibraryImport(LibName)]
    public static partial long sqlite3_last_insert_rowid(nint db);

    [LibraryImport(LibName, StringMarshalling = StringMarshalling.Utf8)]
    public static partial int sqlite3_exec(nint db, string sql, nint callback, nint arg, out nint errmsg);

    [LibraryImport(LibName)]
    public static partial void sqlite3_free(nint ptr);

    [LibraryImport(LibName)]
    public static partial nint sqlite3_errmsg(nint db);
}
