//! SQLite database view through the binary, with databases made by the test.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rusqlite::Connection;

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

/// A fresh empty directory for one test.
fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("sqlite")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_db(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "PRAGMA user_version = 7;
         CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL DEFAULT 1.5, avatar BLOB, bio TEXT);
         CREATE INDEX idx_users_name ON users (name);
         CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);
         CREATE VIEW top_users AS SELECT name FROM users WHERE score > 1;",
    )
    .unwrap();
    for i in 1..=15 {
        conn.execute(
            "INSERT INTO users (name, score, avatar, bio) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                format!("user{i}"),
                f64::from(i) * 1.25,
                vec![0u8; 123],
                "long text ".repeat(100)
            ],
        )
        .unwrap();
    }
    conn.execute("INSERT INTO notes (body) VALUES ('hello\x1b[31mred')", [])
        .unwrap();
}

fn scat(args: &[&str], file: &Path) -> Output {
    Command::new(SCAT)
        .args(args)
        .arg(file)
        .env("COLUMNS", "100")
        .env("NO_COLOR", "1")
        .env("SCAT_CONFIG", "/nonexistent")
        .output()
        .unwrap()
}

fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn renders_tables_schema_and_rows() {
    let d = dir("render");
    let db = d.join("app.db");
    make_db(&db);
    let out = scat(&["--color=never", "-f"], &db);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("page size"), "{text}");
    assert!(text.contains("4,096 B"), "{text}");
    assert!(text.contains("user_version"), "{text}");
    assert!(text.contains("Tables (2 tables, 1 view)"), "{text}");
    assert!(text.contains("15 rows"), "{text}");
    assert!(text.contains("id INTEGER PK"), "{text}");
    assert!(text.contains("name TEXT NOT NULL"), "{text}");
    assert!(text.contains("score REAL DEFAULT 1.5"), "{text}");
    assert!(text.contains("idx_users_name (name)"), "{text}");
    assert!(text.contains("<blob 123 B>"), "{text}");
    assert!(text.contains("user10"), "{text}");
    assert!(!text.contains("user11"), "only 10 sample rows: {text}");
    assert!(text.contains("… 5 more rows"), "{text}");
    assert!(text.contains("top_users"), "{text}");
    assert!(text.contains('…'), "long text is truncated: {text}");
    assert!(
        !text.contains('\x1b'),
        "escapes in cells are neutralized: {text}"
    );
    assert!(text.contains("^[[31mred"), "{text}");
    assert!(
        text.lines().skip(1).all(|l| l.chars().count() <= 100),
        "{text}"
    );
}

#[test]
fn leaves_no_files_next_to_the_database() {
    let d = dir("sidefiles");
    let db = d.join("app.db");
    make_db(&db);
    // A WAL database is the case that would normally create -wal and -shm.
    let conn = Connection::open(&db).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    drop(conn);
    let before = listing(&d);
    let bytes = std::fs::read(&db).unwrap();
    let out = scat(&["--color=never", "-f"], &db);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("WAL"));
    assert_eq!(listing(&d), before);
    assert_eq!(std::fs::read(&db).unwrap(), bytes);
}

#[test]
fn piped_output_is_the_file() {
    let d = dir("piped");
    let db = d.join("app.sqlite3");
    make_db(&db);
    let out = scat(&[], &db);
    assert_eq!(out.stdout, std::fs::read(&db).unwrap());
}

#[test]
fn corrupt_database_is_a_clean_error_and_scat_goes_on() {
    let d = dir("corrupt");
    let bad = d.join("bad.db");
    let mut bytes = b"SQLite format 3\0".to_vec();
    bytes.extend((0..8192u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8));
    std::fs::write(&bad, bytes).unwrap();
    let good = d.join("good.db");
    make_db(&good);
    let out = Command::new(SCAT)
        .args(["--color=never", "-f"])
        .arg(&bad)
        .arg(&good)
        .env("COLUMNS", "100")
        .env("SCAT_CONFIG", "/nonexistent")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bad.db: cannot read database: "), "{err}");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Tables"));
    assert_eq!(listing(&d), ["bad.db", "good.db"]);
}

#[test]
fn hostile_identifiers_are_quoted() {
    let d = dir("hostile");
    let db = d.join("evil.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE \"weird \"\"name\"\"; DROP TABLE victim\" (\"col \"\"x\"\"\" TEXT);
         INSERT INTO \"weird \"\"name\"\"; DROP TABLE victim\" VALUES ('v');
         CREATE TABLE victim (a INTEGER);
         INSERT INTO victim VALUES (1);",
    )
    .unwrap();
    drop(conn);
    let out = scat(&["--color=never", "-f"], &db);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("weird \"name\"; DROP TABLE victim"), "{text}");
    assert!(text.contains("victim"), "{text}");
    let conn = Connection::open(&db).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM victim", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn table_limit_and_row_config() {
    let d = dir("limits");
    let db = d.join("app.db");
    make_db(&db);
    let config = d.join("config.toml");
    std::fs::write(&config, "[sqlite]\nsample_rows = 2\nmax_tables = 1\n").unwrap();
    let out = Command::new(SCAT)
        .args(["--color=never", "-f"])
        .arg(&db)
        .env("COLUMNS", "100")
        .env("SCAT_CONFIG", &config)
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("user2"), "{text}");
    assert!(!text.contains("user3"), "{text}");
    assert!(text.contains("… 2 more tables"), "{text}");
}

#[test]
fn stdin_is_not_rendered() {
    use std::io::Write;
    use std::process::Stdio;
    let d = dir("stdin");
    let db = d.join("app.db");
    make_db(&db);
    let bytes = std::fs::read(&db).unwrap();
    let mut child = Command::new(SCAT)
        .args(["--color=never", "-f"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&bytes).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("Tables"));
}

#[test]
fn one_slow_builtin_is_killed_by_the_wall_clock() {
    // SQLite's progress handler runs between virtual machine steps, and one GLOB over
    // 2 MB of text is a single step that takes far longer than the query budget. The
    // helper is killed after 10 s instead, and the next file is still shown.
    let d = dir("slow");
    let slow = d.join("slow.db");
    let conn = Connection::open(&slow).unwrap();
    conn.execute_batch(
        "CREATE VIEW slow AS SELECT 1 AS x WHERE replace(hex(zeroblob(1000000)),'0','a') \
         GLOB ('*'||printf('%.*c',49000,'a')||'b')",
    )
    .unwrap();
    drop(conn);
    let good = d.join("good.db");
    make_db(&good);
    let started = std::time::Instant::now();
    let out = Command::new(SCAT)
        .args(["--color=never", "-f"])
        .arg(&slow)
        .arg(&good)
        .env("COLUMNS", "100")
        .env("SCAT_CONFIG", "/nonexistent")
        .output()
        .unwrap();
    let elapsed = started.elapsed();
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("slow.db: cannot read database: gave up after 10 s"),
        "{err}"
    );
    assert!(elapsed < std::time::Duration::from_secs(12), "{elapsed:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("users"));
}
