//! Error messages carry text from files (part names, table names). It must never reach
//! the terminal as escape sequences: `report!` sanitizes every message.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const SCAT: &str = env!("CARGO_BIN_EXE_scat");
const ATTACK: &str = "\x1b]0;SCAT_TEST_MARKER\x07";

fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("error-messages-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn stderr_of(path: &std::path::Path) -> Vec<u8> {
    let out = Command::new(SCAT)
        .args(["--no-config", "-f", "--color=never"])
        .arg(path)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap();
    assert!(!out.status.success(), "{out:?}");
    out.stderr
}

fn assert_clean(stderr: &[u8]) {
    let text = String::from_utf8_lossy(stderr);
    assert!(!stderr.contains(&0x1b), "{text:?}");
    assert!(!stderr.contains(&0x07), "{text:?}");
    assert!(text.contains("^[]0;SCAT_TEST_MARKER^G"), "{text:?}");
}

#[test]
fn office_part_name_cannot_inject_escapes() {
    let path = dir("office").join("escape.pptx");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("ppt/presentation.xml", options).unwrap();
    zip.write_all(
        b"<p:presentation xmlns:p=\"p\" xmlns:r=\"r\"><p:sldIdLst>\
          <p:sldId r:id=\"r1\"/></p:sldIdLst></p:presentation>",
    )
    .unwrap();
    zip.start_file("ppt/_rels/presentation.xml.rels", options)
        .unwrap();
    let rels = format!(
        "<Relationships><Relationship Id=\"r1\" Target=\"slides/{ATTACK}.xml\"/></Relationships>"
    );
    zip.write_all(rels.as_bytes()).unwrap();
    zip.finish().unwrap();
    assert_clean(&stderr_of(&path));
}

#[test]
fn sqlite_table_name_cannot_inject_escapes() {
    let path = dir("sqlite").join("escape.db");
    let _ = std::fs::remove_file(&path);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE innocent(x); PRAGMA writable_schema=ON;")
        .unwrap();
    conn.execute(
        "UPDATE sqlite_schema SET name = ?1, sql = ?2 WHERE name = 'innocent'",
        (ATTACK, format!("CREATE TABLE \"{ATTACK}\" (")),
    )
    .unwrap();
    drop(conn);
    assert_clean(&stderr_of(&path));
}
