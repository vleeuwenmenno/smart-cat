//! PDF view through the binary, with small PDFs written by hand.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

/// A valid PDF with one page per entry of `pages`, each showing its text in Helvetica.
fn make_pdf(title: &str, pages: &[&str]) -> Vec<u8> {
    let n = pages.len();
    // Objects: 1 catalog, 2 page tree, 3 font, 4 info, then a page and its content each.
    let mut objects: Vec<String> = Vec::new();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 5 + 2 * i)).collect();
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {n} >>",
        kids.join(" ")
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    objects.push(format!(
        "<< /Title ({title}) /Author (Test Author) /Producer (hand written) >>"
    ));
    for (i, text) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            6 + 2 * i
        ));
        let content = format!("BT /F1 36 Tf 72 720 Td ({text}) Tj ET");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut pdf = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    pdf
}

fn fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pdf");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("doc.pdf"),
        make_pdf("Test Doc", &["Page one", "Page two", "Page three"]),
    )
    .unwrap();
    std::fs::write(
        dir.join("broken.pdf"),
        b"%PDF-1.4\nthis is not a pdf at all\n",
    )
    .unwrap();
    std::fs::write(dir.join("notes.txt"), b"hello\n").unwrap();
    dir
}

fn command(args: &[&str], dir: &Path) -> Command {
    let mut cmd = Command::new(SCAT);
    cmd.args(args)
        .current_dir(dir)
        .env_remove("SCAT_CONFIG")
        .env_remove("SCAT_PROTOCOL")
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("COLUMNS", "40")
        .env("LINES", "30")
        .env("LC_ALL", "C.UTF-8");
    cmd
}

fn run(args: &[&str], dir: &Path) -> Output {
    command(args, dir).output().unwrap()
}

/// Removes SGR sequences (`ESC [ ... m`).
fn strip_sgr(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(pos) = rest.find("\x1b[") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        let end = after.find('m').map_or(after.len(), |e| e + 1);
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn has(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

#[test]
fn piped_output_is_the_raw_file() {
    let dir = fixtures();
    let out = run(&["doc.pdf"], &dir);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, std::fs::read(dir.join("doc.pdf")).unwrap());
}

#[test]
fn header_and_first_page_as_blocks() {
    let dir = fixtures();
    let out = run(
        &["-f", "--protocol=blocks", "--color=always", "doc.pdf"],
        &dir,
    );
    let text = strip_sgr(&String::from_utf8(out.stdout).unwrap());
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.starts_with("doc.pdf\n"), "{text}");
    assert!(text.contains("title      Test Doc"), "{text}");
    assert!(text.contains("author     Test Author"), "{text}");
    assert!(text.contains("pages      3"), "{text}");
    assert!(text.contains("210 x 297 mm (A4)"), "{text}");
    assert!(text.contains("PDF 1.4"), "{text}");
    assert!(text.contains('▀'), "{text}");
    assert!(
        text.trim_end()
            .ends_with("page 1 of 3 — use --pages 2-3 for more"),
        "{text}"
    );
}

#[test]
fn several_pages_get_captions_and_plain_drops_decorations() {
    let dir = fixtures();
    let out = run(
        &[
            "-f",
            "--protocol=blocks",
            "--color=always",
            "--pages",
            "2-3",
            "doc.pdf",
        ],
        &dir,
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains('▀'), "{text}");
    assert!(text.contains("page 2") && text.contains("page 3"), "{text}");
    assert!(text.contains("pages 2-3 of 3 — use --pages all for more"));

    let out = run(&["-f", "--protocol=blocks", "-p", "doc.pdf"], &dir);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        !text.contains("Test Doc") && !text.contains("page 1 of"),
        "{text}"
    );
}

#[test]
fn text_of_selected_pages() {
    if !has("pdftotext") {
        eprintln!("pdftotext not installed; skipped");
        return;
    }
    let dir = fixtures();
    let out = run(&["-f", "--pages", "1,3", "doc.pdf"], &dir);
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(text.contains("Page one"), "{text}");
    assert!(text.contains("Page three"), "{text}");
    assert!(!text.contains("Page two"), "{text}");
    assert!(text.contains("page 3 of 3"), "{text}");
    assert!(!text.contains('\x0c'), "form feeds are removed");
}

#[test]
fn text_from_standard_input() {
    if !has("pdftotext") {
        eprintln!("pdftotext not installed; skipped");
        return;
    }
    let dir = fixtures();
    let mut child = command(&["-f", "--text", "--pages", "all", "-"], &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let pdf = std::fs::read(dir.join("doc.pdf")).unwrap();
    child.stdin.take().unwrap().write_all(&pdf).unwrap();
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(0), "{text}");
    assert!(
        text.contains("Page two") && text.contains("Page three"),
        "{text}"
    );
}

#[test]
fn malformed_pdf_is_an_error_and_scat_continues() {
    let dir = fixtures();
    let out = run(&["-f", "--text", "broken.pdf", "notes.txt"], &dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with("scat: broken.pdf: cannot read PDF"),
        "{stderr}"
    );
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("hello"));
}

#[test]
fn pages_beyond_the_end_and_invalid_specs() {
    let dir = fixtures();
    let out = run(&["-f", "--protocol=ascii", "--pages", "7", "doc.pdf"], &dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no page 7 (the document has 3 pages)"),
        "{stderr}"
    );

    let out = run(&["--pages", "3-1", "doc.pdf"], &dir);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("backwards range"));
}
