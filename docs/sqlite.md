# SQLite databases

Rendering applies in a terminal only. When output is piped or redirected, a database is printed byte for byte like `cat`.

`scat app.db` shows the database instead of its bytes. A file is treated as a database when it starts with `SQLite format 3\0`, whatever its name (`.db`, `.sqlite`, `.sqlite3`, none). A file with a database extension but another content is not touched. Standard input is never shown as a database, since SQLite needs a path.

## What is shown

1. A header: page size, page count, file size, text encoding, journal mode, and `user_version` / `application_id` when set. The journal mode is read from the file header: `WAL` (plus a note when a `-wal` file exists) or the rollback journal.
2. An overview of every table and view with its row count, up to 200 names.
3. For the first 20 tables (views come after tables): the columns as one compact line (`name TYPE PK NOT NULL DEFAULT x`), the indexes (`UNIQUE name (cols)`), and the first 10 rows as a [box table](data-files.md#csv-tsv-and-psv-tables). Rows that are left out show as `… 5 more rows`; tables that are left out as `… 3 more tables`.

```
app.db
  format        SQLite 3
  page size     4,096 B
  pages         9
  file size     36.0 KiB
  encoding      UTF-8
  journal       rollback journal (delete)
  user_version  7

Tables (2 tables, 1 view)
  users      table  15 rows
  notes      table  1 row
  top_users  view   15 rows

users  table 15 rows
  columns   id INTEGER PK, name TEXT NOT NULL, score REAL DEFAULT 1.5, avatar BLOB
  indexes   idx_users_name (name)
┌────┬────────┬───────┬──────────────┐
│ id │ name   │ score │ avatar       │
├────┼────────┼───────┼──────────────┤
│  1 │ user1  │  1.25 │ <blob 123 B> │
│  2 │ user2  │   2.5 │ <blob 123 B> │
└────┴────────┴───────┴──────────────┘
… 13 more rows
```

Cells: `NULL` for null, numbers as they are, BLOBs as `<blob 123 B>`, text cut after 200 characters (`…`) and cut further to fit the terminal width. Line breaks in text show as `↵`. Everything that comes from the file (table, column and index names, defaults, cells) is sanitized: control characters show as `^X` and escape sequences never reach the terminal. Virtual tables are listed but their rows are not read.

## Safety

The database is untrusted input, and looking at it must change nothing.

- It is opened read-only with the `immutable=1` URI parameter. SQLite then takes no locks and creates no `-wal`, `-shm` or journal files. Consequence: changes still sitting in a `-wal` file are not shown; the header says when such a file exists.
- `query_only` is on and `trusted_schema` is off, so views and triggers in the file cannot call risky functions.
- Only fixed queries run: the schema list from `sqlite_schema`, `pragma_table_info` / `pragma_index_list` / `pragma_index_info` with the name as a bound parameter, `SELECT count(*) FROM "name"` and `SELECT * FROM "name" LIMIT n`. Names are quoted as identifiers (double quotes doubled), so a table called `x"; DROP TABLE y` is just a name. SQL stored in the file is never executed by scat itself.
- A progress handler interrupts a query after 2 seconds, and all queries for one file stop after 10 seconds. An interrupted row count shows as `? rows`.
- A locked, truncated or corrupt database gives `scat: NAME: cannot read database: ...` and scat goes on with the next file; the exit status is 1.

## Configuration

```toml
[sqlite]
sample_rows = 10   # rows per table; 0 shows the schema only
max_tables = 20    # tables in detail after the overview; 0 shows the overview only
```

Rules can force or disable the view with `renderer = "database"` (any file, handy for databases without the magic header) or `renderer = "hex"` / `"raw"` to opt out.
