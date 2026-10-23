//! Structured data views through the binary: CSV tables, JSON pretty-printing, cat contract.

use std::path::PathBuf;
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn fixtures() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(make_fixtures).clone()
}

fn make_fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("data");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("a.csv"),
        "name;qty;price\nwidget;3;1.50\n\"gadget, big\";12;20\n",
    )
    .unwrap();
    std::fs::write(dir.join("b.tsv"), "k\tv\nx\t1\ny\t22\n").unwrap();
    let mut big = String::from("n\n");
    for i in 0..500 {
        big.push_str(&format!("{i}\n"));
    }
    std::fs::write(dir.join("big.csv"), big).unwrap();
    std::fs::write(
        dir.join("min.json"),
        "{\"z\":1.0,\"a\":[1e3,12345678901234567890,\"\\u00e9\"],\"e\":{},\"note\":\"padding padding padding padding\"}\n",
    )
    .unwrap();
    std::fs::write(dir.join("pretty.json"), "{\n  \"a\": 1,\n  \"b\": [2]\n}\n").unwrap();
    std::fs::write(
        dir.join("broken.json"),
        "{\"a\":1,\"b\":\"a long enough string to be considered minified json text\",\"c\":",
    )
    .unwrap();
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(fixtures())
        .env_remove("SCAT_CONFIG")
        .env("HOME", fixtures())
        .env_remove("XDG_CONFIG_HOME")
        .env("COLUMNS", "60")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap()
}

fn text(args: &[&str]) -> String {
    let out = run(args);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn csv_table_with_sniffed_delimiter_and_quotes() {
    let got = text(&["-f", "--color=never", "-p", "a.csv"]);
    assert_eq!(
        got,
        "\
┌─────────────┬─────┬───────┐
│ name        │ qty │ price │
├─────────────┼─────┼───────┤
│ widget      │   3 │  1.50 │
│ gadget, big │  12 │    20 │
└─────────────┴─────┴───────┘
"
    );
}

#[test]
fn tsv_table_with_row_numbers() {
    let got = text(&["-f", "--color=never", "b.tsv"]);
    assert_eq!(
        got,
        "\
┌───┬───┬────┐
│   │ k │  v │
├───┼───┼────┤
│ 1 │ x │  1 │
│ 2 │ y │ 22 │
└───┴───┴────┘
"
    );
}

#[test]
fn csv_stops_at_max_rows_with_a_footer() {
    let got = text(&["-f", "--color=never", "-p", "big.csv"]);
    assert_eq!(got.lines().count(), 3 + 200 + 1 + 1);
    assert!(got.ends_with("… 300 more rows\n"), "{got}");
}

#[test]
fn piped_data_files_are_cat() {
    for name in ["a.csv", "b.tsv", "big.csv", "min.json", "broken.json"] {
        let out = run(&[name]);
        let original = std::fs::read(fixtures().join(name)).unwrap();
        assert_eq!(out.stdout, original, "{name}");
    }
}

#[test]
fn minified_json_is_indented_and_exact() {
    let got = text(&["-f", "--color=never", "-p", "min.json"]);
    assert_eq!(
        got,
        "{\n  \"z\": 1.0,\n  \"a\": [\n    1e3,\n    12345678901234567890,\n    \"\\u00e9\"\n  ],\n  \"e\": {},\n  \"note\": \"padding padding padding padding\"\n}\n"
    );
}

#[test]
fn pretty_and_invalid_json_keep_the_plain_view() {
    for name in ["pretty.json", "broken.json"] {
        let got = text(&["-f", "--color=never", "-p", name]);
        let original = std::fs::read_to_string(fixtures().join(name)).unwrap();
        assert_eq!(got, original, "{name}");
    }
}

#[test]
fn forcing_another_language_opts_out() {
    let got = text(&["-f", "--color=never", "-p", "--language=text", "a.csv"]);
    assert!(got.starts_with("name;qty"), "{got}");
}
