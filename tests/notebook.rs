//! Jupyter notebook view through the binary: cells, outputs, escaping, the cat contract
//! and the fallback for files that are not notebooks.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use image::{ImageFormat, Rgba, RgbaImage};
use serde_json::json;

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn png() -> Vec<u8> {
    let img = RgbaImage::from_fn(8, 4, |x, _| {
        if x < 4 {
            Rgba([255, 0, 0, 255])
        } else {
            Rgba([0, 0, 255, 255])
        }
    });
    let mut data = std::io::Cursor::new(Vec::new());
    img.write_to(&mut data, ImageFormat::Png).unwrap();
    data.into_inner()
}

fn notebook() -> String {
    json!({
        "nbformat": 4,
        "nbformat_minor": 5,
        "metadata": {
            "kernelspec": {"display_name": "Python 3", "language": "python", "name": "python3"},
            "language_info": {"name": "python", "version": "3.11.4"}
        },
        "cells": [
            {"cell_type": "markdown", "metadata": {}, "source": ["# Analysis\n", "\n", "Some **bold** words."]},
            {"cell_type": "code", "execution_count": 1, "metadata": {},
             "source": ["print('hi')\n", "x = 1"],
             "outputs": [
                {"output_type": "stream", "name": "stdout", "text": ["hi\n", "there\n"]},
                {"output_type": "execute_result", "execution_count": 1, "metadata": {},
                 "data": {"text/plain": ["42"], "text/html": ["<b>42</b>"]}}
             ]},
            {"cell_type": "code", "execution_count": 2, "metadata": {},
             "source": "1/0",
             "outputs": [{
                "output_type": "error", "ename": "ZeroDivisionError", "evalue": "division by zero",
                "traceback": ["\u{1b}[0;31mZeroDivisionError\u{1b}[0m: division by zero\u{1b}]0;evil\u{7}\u{1b}[2J"]
             }]},
            {"cell_type": "code", "execution_count": 3, "metadata": {},
             "source": "plot()",
             "outputs": [{"output_type": "display_data", "metadata": {},
                          "data": {"image/png": base64(&png()), "text/plain": "<Figure>"}}]},
            {"cell_type": "code", "execution_count": 4, "metadata": {}, "source": "many()",
             "outputs": [{"output_type": "stream", "name": "stdout",
                          "text": (0..250).map(|i| format!("line {i}\n")).collect::<String>()}]},
            {"cell_type": "raw", "metadata": {}, "source": "raw text"}
        ]
    })
    .to_string()
}

fn dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("notebook");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(name: &str, content: &str) -> PathBuf {
    let path = dir().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

fn run(args: &[&str], dir: &Path) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("SCAT_PROTOCOL")
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("COLUMNS", "60")
        .env("LINES", "30")
        .env("COLORTERM", "truecolor")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap()
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn renders_cells_and_outputs() {
    write("a.ipynb", &notebook());
    let out = run(
        &["--color=never", "-f", "--protocol=blocks", "a.ipynb"],
        &dir(),
    );
    let out_text = text(&out);
    assert!(out.status.success(), "{out_text}");
    assert!(
        out_text.starts_with("Jupyter notebook · Python 3 3.11.4 · 6 cells\n"),
        "{out_text}"
    );
    assert!(out_text.contains("Analysis"));
    assert!(out_text.contains("bold"));
    assert!(!out_text.contains("**bold**"));
    assert!(out_text.contains("In [1]:\n  print('hi')\n  x = 1\n"));
    assert!(out_text.contains("Out [1]:\n  hi\n  there\n  42\n"));
    assert!(!out_text.contains("<b>"));
    assert!(out_text.contains("ZeroDivisionError: division by zero"));
    assert!(out_text.contains("In [3]:"));
    assert!(
        out_text.contains("[image/png 8x4]"),
        "no colour, no blocks: {out_text}"
    );
    assert!(!out_text.contains("<Figure>"));
    assert!(out_text.contains("line 199\n"));
    assert!(!out_text.contains("line 200\n"));
    assert!(out_text.contains("… 50 more lines"));
    assert!(out_text.contains("raw text"));
}

#[test]
fn image_without_graphics_is_a_placeholder() {
    write("b.ipynb", &notebook());
    let out = run(
        &["--color=never", "-f", "--protocol=ascii", "b.ipynb"],
        &dir(),
    );
    assert!(text(&out).contains("[image/png 8x4]"), "{}", text(&out));
}

#[test]
fn traceback_escapes_are_neutralised() {
    write("c.ipynb", &notebook());
    let out = run(
        &["--color=always", "-f", "--protocol=blocks", "c.ipynb"],
        &dir(),
    );
    let out_text = text(&out);
    assert!(
        out_text.contains("\u{1b}[0;31mZeroDivisionError"),
        "SGR kept"
    );
    assert!(out_text.contains("^[]0;evil^G"), "{out_text:?}");
    assert!(out_text.contains("^[[2J"));
    assert!(!out_text.contains("\u{1b}]0;evil"));
    assert!(!out_text.contains("\u{1b}[2J"));
    assert!(
        out_text.contains('▀'),
        "image drawn as blocks: {out_text:?}"
    );
    assert!(!out_text.contains("[image/png"));
}

#[test]
fn piped_output_is_the_file() {
    let path = write("d.ipynb", &notebook());
    let out = run(&["d.ipynb"], &dir());
    assert_eq!(out.stdout, std::fs::read(path).unwrap());
}

#[test]
fn renderer_code_shows_the_json() {
    write("e.ipynb", &notebook());
    let config = write(
        "code.toml",
        "[[rule]]\nmatch = { ext = \"ipynb\" }\nrenderer = \"code\"\n",
    );
    let out = Command::new(SCAT)
        .args(["--color=never", "-f", "e.ipynb"])
        .current_dir(dir())
        .env("HOME", dir())
        .env("SCAT_CONFIG", config)
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap();
    let out_text = text(&out);
    assert!(out_text.contains("\"nbformat\":4"), "{out_text}");
    assert!(!out_text.contains("Jupyter notebook"));
}

#[test]
fn malformed_notebook_falls_back_to_json() {
    write("f.ipynb", "{\"nbformat\":4, \"cells\": [");
    let out = run(&["--color=never", "-f", "f.ipynb"], &dir());
    assert!(text(&out).contains("\"cells\""));
    let note = String::from_utf8_lossy(&out.stderr);
    assert!(note.contains("showing JSON"), "{note}");

    write("g.ipynb", "{\"hello\": 1}");
    let out = run(&["--color=never", "-f", "g.ipynb"], &dir());
    assert!(text(&out).contains("\"hello\""));
    assert!(String::from_utf8_lossy(&out.stderr).contains("showing JSON"));
}
