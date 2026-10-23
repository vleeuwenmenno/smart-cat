//! Office documents through the binary: docx, odt, pptx, odp, xlsx and ods fixtures
//! written in the test, rendered output, the cat contract and malformed files.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use zip::write::SimpleFileOptions;

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn zip_file(path: &Path, parts: &[(&str, &str)]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    for (name, body) in parts {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
    xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

fn docx(path: &Path) {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document {W}><w:body>
<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Quarterly report</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Plain, </w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>bold</w:t></w:r><w:r><w:t xml:space="preserve"> and </w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t>italic</w:t></w:r><w:r><w:t xml:space="preserve"> &amp; </w:t></w:r><w:hyperlink r:id="rId9"><w:r><w:t>a link</w:t></w:r></w:hyperlink></w:p>
<w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Details</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>first bullet</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>second bullet</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="2"/></w:numPr></w:pPr><w:r><w:t>step one</w:t></w:r></w:p>
<w:tbl>
<w:tr><w:tc><w:p><w:r><w:t>Name</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Score</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:p><w:r><w:t>Ada</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>42</w:t></w:r></w:p></w:tc></w:tr>
</w:tbl>
<w:p><w:r><w:t>Hostile </w:t></w:r><w:r><w:t>&#27;]0;owned&#7;text</w:t></w:r></w:p>
</w:body></w:document>"#
    );
    let numbering = format!(
        r#"<w:numbering {W}>
<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/></w:lvl></w:abstractNum>
<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
<w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
</w:numbering>"#
    );
    let styles = format!(
        r#"<w:styles {W}>
<w:style w:styleId="Heading1"><w:name w:val="heading 1"/></w:style>
<w:style w:styleId="Heading2"><w:name w:val="heading 2"/></w:style>
</w:styles>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId9" Type="hyperlink" Target="https://example.com/x" TargetMode="External"/>
</Relationships>"#;
    zip_file(
        path,
        &[
            ("[Content_Types].xml", "<Types/>"),
            ("word/document.xml", &document),
            ("word/numbering.xml", &numbering),
            ("word/styles.xml", &styles),
            ("word/_rels/document.xml.rels", rels),
            (
                "docProps/app.xml",
                "<Properties><Pages>3</Pages></Properties>",
            ),
        ],
    );
}

fn pptx(path: &Path) {
    const P: &str = "xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" \
        xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
        xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
    let presentation = format!(
        r#"<p:presentation {P}><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId1"/></p:sldIdLst></p:presentation>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="slide" Target="slides/slide1.xml"/>
<Relationship Id="rId2" Type="slide" Target="slides/slide2.xml"/>
</Relationships>"#;
    // Presentation order is slide2 first, then slide1.
    let slide2 = format!(
        r#"<p:sld {P}><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Welcome</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:txBody>
<a:p><a:r><a:rPr b="1"/><a:t>point A</a:t></a:r></a:p><a:p><a:r><a:t>point B</a:t></a:r></a:p></p:txBody></p:sp>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="sldNum"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>99</a:t></a:r></a:p></p:txBody></p:sp>
</p:spTree></p:cSld></p:sld>"#
    );
    let slide1 = format!(
        r#"<p:sld {P}><p:cSld><p:spTree>
<p:sp><p:nvSpPr><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Numbers</a:t></a:r></a:p></p:txBody></p:sp>
<p:graphicFrame><a:graphic><a:graphicData><a:tbl>
<a:tr><a:tc><a:txBody><a:p><a:r><a:t>Q</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>Total</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
<a:tr><a:tc><a:txBody><a:p><a:r><a:t>Q1</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:p><a:r><a:t>7</a:t></a:r></a:p></a:txBody></a:tc></a:tr>
</a:tbl></a:graphicData></a:graphic></p:graphicFrame>
</p:spTree></p:cSld></p:sld>"#
    );
    zip_file(
        path,
        &[
            ("[Content_Types].xml", "<Types/>"),
            ("ppt/presentation.xml", &presentation),
            ("ppt/_rels/presentation.xml.rels", rels),
            ("ppt/slides/slide1.xml", &slide1),
            ("ppt/slides/slide2.xml", &slide2),
        ],
    );
}

const ODF_NS: &str = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
    xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
    xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
    xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
    xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" \
    xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" \
    xmlns:presentation=\"urn:oasis:names:tc:opendocument:xmlns:presentation:1.0\" \
    xmlns:xlink=\"http://www.w3.org/1999/xlink\"";

fn odt(path: &Path) {
    let content = format!(
        r#"<office:document-content {ODF_NS}>
<office:automatic-styles>
<style:style style:name="T1" style:family="text"><style:text-properties fo:font-weight="bold"/></style:style>
<text:list-style style:name="L1"><text:list-level-style-number text:level="1"/></text:list-style>
</office:automatic-styles>
<office:body><office:text>
<text:h text:outline-level="1">Meeting notes</text:h>
<text:p>Some <text:span text:style-name="T1">important</text:span> text.</text:p>
<text:list text:style-name="L1"><text:list-item><text:p>one</text:p></text:list-item><text:list-item><text:p>two</text:p></text:list-item></text:list>
<table:table><table:table-row><table:table-cell><text:p>K</text:p></table:table-cell><table:table-cell><text:p>V</text:p></table:table-cell></table:table-row>
<table:table-row><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell><text:p>1</text:p></table:table-cell></table:table-row></table:table>
</office:text></office:body></office:document-content>"#
    );
    zip_file(
        path,
        &[
            ("mimetype", "application/vnd.oasis.opendocument.text"),
            ("content.xml", &content),
        ],
    );
}

fn odp(path: &Path) {
    let content = format!(
        r#"<office:document-content {ODF_NS}><office:body><office:presentation>
<draw:page draw:name="a"><draw:frame presentation:class="title"><draw:text-box><text:p>First</text:p></draw:text-box></draw:frame>
<draw:frame><draw:text-box><text:p>body text</text:p></draw:text-box></draw:frame></draw:page>
<draw:page draw:name="b"><draw:frame><draw:text-box><text:p>second slide</text:p></draw:text-box></draw:frame></draw:page>
</office:presentation></office:body></office:document-content>"#
    );
    zip_file(
        path,
        &[
            (
                "mimetype",
                "application/vnd.oasis.opendocument.presentation",
            ),
            ("content.xml", &content),
        ],
    );
}

const ROOT_RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#;

fn xlsx(path: &Path) {
    let mut rows = String::from(
        r#"<row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c><c r="B1" t="inlineStr"><is><t>Qty</t></is></c></row>"#,
    );
    for i in 2..=6 {
        rows.push_str(&format!(
            r#"<row r="{i}"><c r="A{i}" t="inlineStr"><is><t>thing {i}</t></is></c><c r="B{i}"><v>{}</v></c></row>"#,
            i * 10
        ));
    }
    let sheet1 = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#
    );
    let sheet2 = r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>
<row r="1"><c r="A1" t="inlineStr"><is><t>Only</t></is></c></row></sheetData></worksheet>"#;
    let workbook = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>
<sheet name="Inventory" sheetId="1" r:id="rId1"/><sheet name="Notes" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;
    zip_file(
        path,
        &[
            ("[Content_Types].xml", "<Types/>"),
            ("_rels/.rels", ROOT_RELS),
            ("xl/workbook.xml", workbook),
            ("xl/_rels/workbook.xml.rels", rels),
            ("xl/worksheets/sheet1.xml", &sheet1),
            ("xl/worksheets/sheet2.xml", sheet2),
        ],
    );
}

/// Written once per test binary: tests run in parallel and must not rewrite files
/// another test is reading.
fn fixtures() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(make_fixtures).clone()
}

fn make_fixtures() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("office");
    std::fs::create_dir_all(&dir).unwrap();
    docx(&dir.join("report.docx"));
    pptx(&dir.join("deck.pptx"));
    odt(&dir.join("notes.odt"));
    odp(&dir.join("slides.odp"));
    xlsx(&dir.join("stock.xlsx"));
    std::fs::write(dir.join("bad.docx"), b"PK\x03\x04 this is not really a zip").unwrap();
    // A zip that is not a document at all.
    zip_file(&dir.join("hollow.docx"), &[("readme.txt", "hi")]);
    std::fs::write(
        dir.join("cut.xlsx"),
        b"PK\x03\x04\x14\0\0\0\x08\0 cut short",
    )
    .unwrap();
    dir
}

fn run(args: &[&str], dir: &Path) -> Output {
    Command::new(SCAT)
        .args(args)
        .current_dir(dir)
        .env_remove("SCAT_CONFIG")
        .env("HOME", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("COLUMNS", "80")
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap()
}

fn rendered(dir: &Path, file: &str) -> String {
    let out = run(&["-f", "--color=never", file], dir);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn docx_renders_as_markdown() {
    let dir = fixtures();
    let text = rendered(&dir, "report.docx");
    assert!(text.starts_with("Word document (docx), 3 pages"), "{text}");
    assert!(text.contains("Quarterly report"), "{text}");
    assert!(text.contains("Details"), "{text}");
    assert!(text.contains("bold") && text.contains("italic"), "{text}");
    assert!(text.contains("a link"), "{text}");
    assert!(text.contains("first bullet") && text.contains("second bullet"));
    assert!(text.contains("step one"));
    assert!(text.contains("Name") && text.contains("Ada") && text.contains("42"));
    assert!(text.contains('┌') || text.contains('|'), "{text}");
    // Document text never carries escape sequences into the output.
    assert!(!text.contains('\x1b') && !text.contains('\x07'), "{text:?}");
}

#[test]
fn docx_with_colours_is_styled() {
    let dir = fixtures();
    let out = run(&["-f", "--color=always", "report.docx"], &dir);
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("\x1b[2mWord document"), "{text:?}");
}

#[test]
fn pptx_has_slide_headings_in_presentation_order() {
    let dir = fixtures();
    let text = rendered(&dir, "deck.pptx");
    assert!(text.starts_with("PowerPoint presentation (pptx), 2 slides"));
    let slide1 = text.find("Slide 1").expect("Slide 1");
    let slide2 = text.find("Slide 2").expect("Slide 2");
    let welcome = text.find("Welcome").expect("Welcome");
    let numbers = text.find("Numbers").expect("Numbers");
    assert!(
        slide1 < welcome && welcome < slide2 && slide2 < numbers,
        "{text}"
    );
    assert!(text.contains("point A") && text.contains("point B"));
    assert!(text.contains("Total") && text.contains("Q1"));
    assert!(
        !text.contains("99"),
        "slide number placeholder shown: {text}"
    );
}

#[test]
fn odt_and_odp_render() {
    let dir = fixtures();
    let text = rendered(&dir, "notes.odt");
    assert!(text.starts_with("OpenDocument text (odt)"), "{text}");
    assert!(text.contains("Meeting notes") && text.contains("important"));
    assert!(text.contains("one") && text.contains("two"));
    assert!(text.contains('K') && text.contains('V'));
    let text = rendered(&dir, "slides.odp");
    assert!(text.contains("2 slides"), "{text}");
    assert!(text.contains("Slide 1") && text.contains("Slide 2"));
    assert!(text.contains("First") && text.contains("second slide"));
}

#[test]
fn xlsx_has_a_table_per_sheet_and_caps_rows() {
    let dir = fixtures();
    let text = rendered(&dir, "stock.xlsx");
    assert!(
        text.starts_with("Excel workbook (xlsx), 2 sheets"),
        "{text}"
    );
    assert!(text.contains("Inventory") && text.contains("Notes"));
    assert!(text.contains("thing 6") && text.contains("60"), "{text}");

    let config = dir.join("capped.toml");
    std::fs::write(&config, "[office]\nmax_rows = 2\n").unwrap();
    let out = run(
        &[
            "-f",
            "--color=never",
            "--config",
            config.to_str().unwrap(),
            "stock.xlsx",
        ],
        &dir,
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("thing 3"), "{text}");
    assert!(!text.contains("thing 4"), "{text}");
    assert!(text.contains("… 3 more rows"), "{text}");
}

#[test]
fn plain_drops_the_header_line() {
    let dir = fixtures();
    let out = run(&["-f", "--color=never", "--plain", "report.docx"], &dir);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(!text.contains("Word document"), "{text}");
    assert!(text.contains("Quarterly report"));
}

#[test]
fn piped_output_is_the_input() {
    let dir = fixtures();
    for name in [
        "report.docx",
        "deck.pptx",
        "notes.odt",
        "slides.odp",
        "stock.xlsx",
    ] {
        let out = run(&[name], &dir);
        assert!(out.status.success());
        assert_eq!(out.stdout, std::fs::read(dir.join(name)).unwrap(), "{name}");
    }
}

#[test]
fn corrupt_documents_give_a_clean_error_and_scat_continues() {
    let dir = fixtures();
    for name in ["bad.docx", "hollow.docx", "cut.xlsx"] {
        let out = run(&["-f", "--color=never", name, "notes.odt"], &dir);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.starts_with(&format!("scat: {name}: cannot read document: ")),
            "{err}"
        );
        assert!(!err.contains("panicked"), "{err}");
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stdout).contains("Meeting notes"));
    }
}

#[test]
fn archive_and_hex_renderers_still_show_the_zip() {
    let dir = fixtures();
    let config = dir.join("hex.toml");
    std::fs::write(
        &config,
        "[[rule]]\nmatch = { ext = \"docx\" }\nrenderer = \"hex\"\n",
    )
    .unwrap();
    let out = run(
        &[
            "-f",
            "--color=never",
            "--config",
            config.to_str().unwrap(),
            "report.docx",
        ],
        &dir,
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("PK"), "{text}");
    assert!(!text.contains("Word document"));
}
