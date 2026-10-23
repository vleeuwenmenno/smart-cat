//! Certificate and key view: X.509 certificates and requests, public keys, OpenSSH
//! key files, and private keys with their key material hidden.
//!
//! PEM files are scanned block by block. Each recognised block is replaced by its
//! summary; text around the blocks is printed in place (sanitized). Private key blocks
//! are never printed here, whether they parse or not.

mod der;
mod keys;
pub mod redact;
mod x509;

use std::io::{self, Write};
use std::ops::Range;

use crate::render::sanitize_line;

/// Files larger than this are not parsed (they are shown as text).
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// The note under every hidden private key.
pub const HIDDEN_NOTE: &str = "private key material hidden; use --raw or a pipe to print it";

/// Colour of a value in a summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Red,
    Yellow,
    Green,
}

/// One `key  value` line. Values may come from the file; they are sanitized on output.
pub struct Row {
    pub key: &'static str,
    pub value: String,
    pub tone: Tone,
}

impl Row {
    pub fn new(key: &'static str, value: String) -> Self {
        Self::toned(key, value, Tone::Plain)
    }

    pub fn toned(key: &'static str, value: String, tone: Tone) -> Self {
        Self { key, value, tone }
    }
}

/// A titled block of rows, with an optional dim note under it.
pub struct Summary {
    pub title: String,
    pub rows: Vec<Row>,
    pub note: Option<String>,
}

pub struct Options {
    pub colors: bool,
    pub utf8: bool,
    pub tab_width: usize,
    /// Current time in Unix seconds, for the validity colours.
    pub now: i64,
}

/// File name extensions that may hold PEM text or DER. Other files (source code with an
/// embedded test certificate, YAML, ...) keep their own view.
const EXTENSIONS: &[&str] = &[
    "pem", "crt", "cer", "cert", "der", "key", "pub", "csr", "req", "p8", "pk8", "txt",
];

/// PEM labels with a summary. Any label containing `PRIVATE KEY` is also claimed, so
/// that it is hidden.
const LABELS: &[&str] = &[
    "CERTIFICATE",
    "TRUSTED CERTIFICATE",
    "X509 CERTIFICATE",
    "CERTIFICATE REQUEST",
    "NEW CERTIFICATE REQUEST",
    "PUBLIC KEY",
    "RSA PUBLIC KEY",
];

fn extension(file_name: Option<&str>) -> Option<String> {
    let name = file_name?;
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty()).then(|| ext.to_ascii_lowercase())
}

/// SSH key list files by name: `*.pub`, `authorized_keys*`, `known_hosts*`.
fn ssh_named(file_name: Option<&str>) -> bool {
    file_name.is_some_and(|n| {
        n.starts_with("authorized_keys") || n.starts_with("known_hosts") || n.ends_with(".pub")
    })
}

/// Whether the `auto` renderer may show this file as certificates or keys at all,
/// judged by name only: a known extension, an SSH key list name, or no extension.
/// Text shown for such a file by `auto` gets its private key blocks hidden.
pub fn eligible(file_name: Option<&str>) -> bool {
    ssh_named(file_name) || extension(file_name).is_none_or(|e| EXTENSIONS.contains(&e.as_str()))
}

fn claimed_label(label: &str) -> bool {
    LABELS.contains(&label) || label.contains("PRIVATE KEY")
}

/// Whether the `auto` renderer shows this file as certificates or keys: a known PEM
/// block at the start of a line in the first bytes, an OpenSSH public key line, or
/// (binary) a `.der`/`.cer`/`.crt` file that parses as a certificate.
pub fn wants(file_name: Option<&str>, head: &[u8]) -> bool {
    let ext = extension(file_name);
    // `infer` calls PEM text `application/x-x509-ca-cert` (binary), so decide here.
    let known_name = ssh_named(file_name);
    let name_ok = known_name || ext.as_deref().is_none_or(|e| EXTENSIONS.contains(&e));
    // PEM text with a stray byte that is not UTF-8 still counts as PEM.
    let has_block = || {
        String::from_utf8_lossy(head)
            .lines()
            .any(|line| begin_label(line).is_some_and(claimed_label))
    };
    if is_binary(head) {
        if name_ok && !head.contains(&0) && has_block() {
            return true;
        }
        // Parsed in full later; a file that does not parse gets the hex dump.
        return matches!(ext.as_deref(), Some("der" | "cer" | "crt"))
            && head.first() == Some(&der::SEQUENCE);
    }
    if !name_ok {
        return false;
    }
    if has_block() {
        return true;
    }
    let text = String::from_utf8_lossy(head);
    // The first line that is not blank or a comment must be a key line.
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .is_some_and(|l| keys::parse_ssh_line(l).is_some())
}

/// DER rather than text: a NUL byte, or bytes that are not UTF-8 (a cut-off character
/// at the end of the sniffed head is fine).
pub fn is_binary(data: &[u8]) -> bool {
    data.contains(&0) || std::str::from_utf8(data).is_err_and(|e| e.error_len().is_some())
}

fn begin_label(line: &str) -> Option<&str> {
    line.trim_end()
        .strip_prefix("-----BEGIN ")?
        .strip_suffix("-----")
}

/// Renders `data`. Returns `false` when nothing was recognised, so the caller can show
/// the file as before; nothing has been written then.
pub fn render<W: Write>(
    out: &mut W,
    data: &[u8],
    file_name: Option<&str>,
    opts: &Options,
) -> io::Result<bool> {
    let text = match std::str::from_utf8(data) {
        Ok(text) => std::borrow::Cow::Borrowed(text),
        Err(_) => {
            if let Some(summary) = der_summary(data, opts.now) {
                write_summary(out, &summary, opts)?;
                return Ok(true);
            }
            // PEM text with a stray byte that is not UTF-8: still a text file.
            if data.contains(&0) {
                return Ok(false);
            }
            String::from_utf8_lossy(data)
        }
    };
    let text = text.as_ref();
    if let Some(entries) = ssh_entries(text, file_name) {
        write_ssh(out, &entries, file_name, opts)?;
        return Ok(true);
    }
    let all: Vec<&str> = text.lines().collect();
    // What each line shows when it is printed as text: private key bodies are hidden
    // wherever they are, also inside blocks that did not parse or that a nested BEGIN
    // cut short. The scanner sees every line, so its state follows the whole file.
    let note = format!("({HIDDEN_NOTE})");
    let mut scanner = redact::Scanner::new();
    let shown: Vec<Vec<String>> = all
        .iter()
        .map(|line| {
            let mut out = Vec::new();
            scanner.push(line.as_bytes(), &note, &mut out);
            if out.is_empty() {
                // A blank line, or a hidden one.
                return if scanner.hiding() {
                    Vec::new()
                } else {
                    vec![String::new()]
                };
            }
            let out = out.strip_suffix(b"\n").unwrap_or(&out);
            out.split(|&b| b == b'\n')
                .map(|piece| String::from_utf8_lossy(piece).into_owned())
                .collect()
        })
        .collect();
    let segments = pem_segments(&all);
    let mut certs = Vec::new();
    let mut parts = Vec::new();
    let mut recognised = false;
    for segment in segments {
        match segment {
            Segment::Text(lines) => parts.push(Part::Text(lines)),
            Segment::Block(block) => match describe_block(&block, opts.now) {
                Described::Cert(cert) => {
                    recognised = true;
                    certs.push(cert);
                    parts.push(Part::Cert(certs.len() - 1));
                }
                Described::Summary(summary) => {
                    recognised = true;
                    parts.push(Part::Summary(summary));
                }
                Described::Unknown => parts.push(Part::Text(block.lines)),
            },
        }
    }
    if !recognised {
        return Ok(false);
    }
    let total = certs.len();
    // Summaries get one blank line around them.
    let mut last = Last::Nothing;
    for part in &parts {
        if let Part::Text(range) = part {
            for line in range.clone().flat_map(|idx| &shown[idx]) {
                let blank = line.trim().is_empty();
                if last == Last::Summary {
                    if blank {
                        continue;
                    }
                    out.write_all(b"\n")?;
                }
                writeln!(out, "{}", sanitize_line(line, opts.tab_width))?;
                last = if blank { Last::Blank } else { Last::Text };
            }
            continue;
        }
        if matches!(last, Last::Text | Last::Summary) {
            out.write_all(b"\n")?;
        }
        write_part(out, part, &certs, total, opts)?;
        last = Last::Summary;
    }
    if let Some(chain) = chain_line(&certs, opts.utf8) {
        out.write_all(b"\n")?;
        let (dim, reset) = dim(opts.colors);
        writeln!(
            out,
            "{dim}chain: {}{reset}",
            crate::render::sanitize_str(&chain)
        )?;
    }
    Ok(true)
}

fn write_part<W: Write>(
    out: &mut W,
    part: &Part,
    certs: &[x509::Cert],
    total: usize,
    opts: &Options,
) -> io::Result<()> {
    match part {
        Part::Cert(idx) => {
            let cert = &certs[*idx];
            let title = if total > 1 {
                format!("Certificate {} of {total}", idx + 1)
            } else {
                cert.summary.title.clone()
            };
            write_rows(out, &title, &cert.summary.rows, opts)
        }
        Part::Summary(summary) => write_summary(out, summary, opts),
        Part::Text(_) => Ok(()),
    }
}

/// What the last printed line was.
#[derive(PartialEq, Eq)]
enum Last {
    Nothing,
    Blank,
    Text,
    Summary,
}

enum Part {
    Text(Range<usize>),
    Cert(usize),
    Summary(Summary),
}

/// `chain: #1 leaf → #2 intermediate → #3 root` for adjacent certificates whose issuer
/// and subject match; runs that do not link are separated by `;`.
fn chain_line(certs: &[x509::Cert], utf8: bool) -> Option<String> {
    if certs.len() < 2 {
        return None;
    }
    let arrow = if utf8 { " → " } else { " -> " };
    let mut linked = false;
    let mut text = String::new();
    for (idx, cert) in certs.iter().enumerate() {
        if idx > 0 {
            if certs[idx - 1].issuer == cert.subject {
                linked = true;
                text.push_str(arrow);
            } else {
                text.push_str("; ");
            }
        }
        text.push_str(&format!("#{} {}", idx + 1, cert.short_name));
        if cert.self_signed {
            text.push_str(" (self-signed)");
        }
    }
    linked.then_some(text)
}

/// A DER file: a certificate, a request, a public key or a PKCS#8 private key.
fn der_summary(data: &[u8], now: i64) -> Option<Summary> {
    if let Some(cert) = x509::certificate(data, now) {
        return Some(cert.summary);
    }
    if let Some(summary) = x509::request(data) {
        return Some(summary);
    }
    if let Some(key) = keys::describe_spki(data) {
        return Some(public_summary(
            "Public key",
            &key,
            "SubjectPublicKeyInfo (DER)",
        ));
    }
    None
}

struct Block<'a> {
    label: &'a str,
    headers: Vec<(String, String)>,
    /// Base64 body; `None` when the block has no END line.
    body: Option<Vec<u8>>,
    /// Indices of the original lines, for blocks that are shown as text.
    lines: Range<usize>,
}

enum Segment<'a> {
    Text(Range<usize>),
    Block(Block<'a>),
}

/// Splits lines into PEM blocks and the lines between them. A block without an END
/// line runs to the next BEGIN line or the end of the text, so a block nested in an
/// unfinished one (a private key inside a cut-off certificate) is still its own block.
fn pem_segments<'a>(lines: &[&'a str]) -> Vec<Segment<'a>> {
    let mut segments = Vec::new();
    let mut plain_start = 0;
    let mut idx = 0;
    while idx < lines.len() {
        let Some(label) = begin_label(lines[idx]) else {
            idx += 1;
            continue;
        };
        if plain_start < idx {
            segments.push(Segment::Text(plain_start..idx));
        }
        let start = idx;
        idx += 1;
        let end = format!("-----END {label}-----");
        let mut headers = Vec::new();
        let mut body = String::new();
        let mut in_headers = true;
        let mut closed = false;
        while idx < lines.len() {
            let inner = lines[idx];
            if begin_label(inner).is_some() {
                break;
            }
            idx += 1;
            let trimmed = inner.trim();
            if trimmed == end {
                closed = true;
                break;
            }
            if in_headers {
                if let Some((key, value)) = trimmed.split_once(':') {
                    headers.push((key.trim().to_string(), value.trim().to_string()));
                    continue;
                }
                in_headers = false;
                if trimmed.is_empty() {
                    continue;
                }
            }
            body.push_str(trimmed);
        }
        segments.push(Segment::Block(Block {
            label,
            headers,
            body: if closed {
                der::base64_decode(body.as_bytes())
            } else {
                None
            },
            lines: start..idx,
        }));
        plain_start = idx;
    }
    if plain_start < lines.len() {
        segments.push(Segment::Text(plain_start..lines.len()));
    }
    segments
}

enum Described {
    Cert(x509::Cert),
    Summary(Summary),
    /// Shown as the original text.
    Unknown,
}

fn describe_block(block: &Block, now: i64) -> Described {
    let body = block.body.as_deref();
    if block.label.contains("PRIVATE KEY") {
        return Described::Summary(private_summary(block.label, &block.headers, body));
    }
    let Some(body) = body else {
        return Described::Unknown;
    };
    match block.label {
        "CERTIFICATE" | "X509 CERTIFICATE" => {
            x509::certificate(body, now).map_or(Described::Unknown, Described::Cert)
        }
        // OpenSSL's TRUSTED CERTIFICATE has trust settings after the certificate.
        "TRUSTED CERTIFICATE" => der::read(body)
            .and_then(|(_, rest)| x509::certificate(&body[..body.len() - rest.len()], now))
            .map_or(Described::Unknown, Described::Cert),
        "CERTIFICATE REQUEST" | "NEW CERTIFICATE REQUEST" => {
            x509::request(body).map_or(Described::Unknown, Described::Summary)
        }
        "PUBLIC KEY" => keys::describe_spki(body).map_or(Described::Unknown, |k| {
            Described::Summary(public_summary("Public key", &k, "SubjectPublicKeyInfo"))
        }),
        "RSA PUBLIC KEY" => keys::describe_rsa_public(body).map_or(Described::Unknown, |k| {
            Described::Summary(public_summary("Public key", &k, "PKCS#1"))
        }),
        _ => Described::Unknown,
    }
}

fn public_summary(title: &str, key: &keys::KeyInfo, format: &str) -> Summary {
    let mut rows = vec![
        Row::new("key", key.summary()),
        Row::new("format", format.to_string()),
    ];
    if let Some(fp) = &key.fingerprint {
        rows.push(Row::new("fingerprint", fp.clone()));
    }
    Summary {
        title: title.into(),
        rows,
        note: None,
    }
}

fn private_summary(label: &str, headers: &[(String, String)], body: Option<&[u8]>) -> Summary {
    let info = keys::describe_private(label, headers, body);
    let mut rows = vec![Row::new("key", info.key.summary())];
    let format = if info.format == "unknown" {
        label.to_string()
    } else {
        info.format.to_string()
    };
    rows.push(Row::new("format", format));
    rows.push(Row::new(
        "encrypted",
        match &info.encryption {
            Some(scheme) if scheme != "yes" => format!("yes, {scheme}"),
            Some(_) => "yes".into(),
            None if body.is_none() => "unknown (could not be read)".into(),
            None => "no".into(),
        },
    ));
    if let Some(fp) = &info.key.fingerprint {
        rows.push(Row::new("fingerprint", format!("{fp} (public key)")));
    }
    if let Some(comment) = info.comment {
        rows.push(Row::new("comment", comment));
    }
    Summary {
        title: "Private key".into(),
        rows,
        note: Some(HIDDEN_NOTE.into()),
    }
}

/// One line of an SSH key list: a key, or a line that is not one.
enum SshEntry {
    Key(keys::SshLine),
    Other(usize),
}

/// Parses an OpenSSH public key file, `authorized_keys` or `known_hosts`. `None` when
/// no line is a key (or the text holds PEM blocks).
fn ssh_entries(text: &str, file_name: Option<&str>) -> Option<Vec<SshEntry>> {
    if text.lines().any(|l| begin_label(l).is_some()) {
        return None;
    }
    let mut entries = Vec::new();
    let mut keys = 0;
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        match keys::parse_ssh_line(trimmed) {
            Some(key) => {
                keys += 1;
                entries.push(SshEntry::Key(key));
            }
            None => entries.push(SshEntry::Other(idx + 1)),
        }
    }
    let first_is_key = matches!(entries.first(), Some(SshEntry::Key(_)));
    (keys > 0 && (first_is_key || ssh_named(file_name))).then_some(entries)
}

/// Whether the text before the key type is host patterns (known_hosts) rather than
/// authorized_keys options.
fn prefix_is_hosts(prefix: &str, file_name: Option<&str>) -> bool {
    match file_name {
        Some(n) if n.starts_with("known_hosts") => true,
        Some(n) if n.starts_with("authorized_keys") => false,
        _ => {
            !(prefix.contains('=')
                || ["restrict", "cert-authority", "no-", "permit"]
                    .iter()
                    .any(|o| prefix.starts_with(o)))
        }
    }
}

/// Host patterns with hashed entries (`|1|salt|hash`) shown as `(hashed)`.
fn hosts(prefix: &str) -> String {
    prefix
        .split_whitespace()
        .map(|word| {
            word.split(',')
                .map(|h| if h.starts_with("|1|") { "(hashed)" } else { h })
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_ssh<W: Write>(
    out: &mut W,
    entries: &[SshEntry],
    file_name: Option<&str>,
    opts: &Options,
) -> io::Result<()> {
    // A single plain key (an id_*.pub file) gets a table.
    if let [SshEntry::Key(line)] = entries
        && line.prefix.is_none()
    {
        let mut rows = vec![
            Row::new("type", line.key.algorithm.clone()),
            Row::new(
                "bits",
                line.key.bits.map(|b| b.to_string()).unwrap_or_default(),
            ),
        ];
        if let Some(curve) = line.key.curve {
            rows.push(Row::new("curve", curve.into()));
        }
        rows.push(Row::new(
            "fingerprint",
            line.key.fingerprint.clone().unwrap_or_default(),
        ));
        rows.push(Row::new(
            "comment",
            line.comment.clone().unwrap_or_default(),
        ));
        return write_rows(out, "SSH public key", &rows, opts);
    }
    let (bold, reset) = if opts.colors {
        ("\x1b[1m", "\x1b[0m")
    } else {
        ("", "")
    };
    let (dim, undim) = dim(opts.colors);
    let count = entries
        .iter()
        .filter(|e| matches!(e, SshEntry::Key(_)))
        .count();
    let noun = if count == 1 { "key" } else { "keys" };
    writeln!(
        out,
        "{bold}SSH public keys{reset} {dim}({count} {noun}){undim}"
    )?;
    let type_width = entries
        .iter()
        .filter_map(|e| match e {
            SshEntry::Key(k) => Some(k.key.algorithm.len()),
            SshEntry::Other(_) => None,
        })
        .max()
        .unwrap_or(0);
    for entry in entries {
        let line = match entry {
            SshEntry::Other(number) => {
                writeln!(out, "  {dim}line {number}: not a key{undim}")?;
                continue;
            }
            SshEntry::Key(line) => line,
        };
        let bits = line.key.bits.map(|b| b.to_string()).unwrap_or_default();
        let fp = line.key.fingerprint.as_deref().unwrap_or("");
        let mut who = Vec::new();
        let mut options = None;
        match &line.prefix {
            Some(prefix) if prefix_is_hosts(prefix, file_name) => who.push(hosts(prefix)),
            Some(prefix) => options = Some(prefix.as_str()),
            None => {}
        }
        if let Some(comment) = &line.comment {
            who.push(comment.clone());
        }
        let who = crate::render::sanitize_str(&who.join("  "));
        write!(
            out,
            "  {:<type_width$} {bits:>4}  {fp}  {who}",
            line.key.algorithm
        )?;
        if let Some(options) = options {
            write!(
                out,
                "  {dim}options: {}{undim}",
                crate::render::sanitize_str(options)
            )?;
        }
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn dim(colors: bool) -> (&'static str, &'static str) {
    if colors {
        ("\x1b[2m", "\x1b[0m")
    } else {
        ("", "")
    }
}

fn write_summary<W: Write>(out: &mut W, summary: &Summary, opts: &Options) -> io::Result<()> {
    write_rows(out, &summary.title, &summary.rows, opts)?;
    if let Some(note) = &summary.note {
        let (dim, reset) = dim(opts.colors);
        writeln!(out, "  {dim}{note}{reset}")?;
    }
    Ok(())
}

/// A bold title and aligned rows with dim keys. Values are sanitized here.
fn write_rows<W: Write>(out: &mut W, title: &str, rows: &[Row], opts: &Options) -> io::Result<()> {
    let (bold, dim, reset) = if opts.colors {
        ("\x1b[1m", "\x1b[2m", "\x1b[0m")
    } else {
        ("", "", "")
    };
    writeln!(out, "{bold}{}{reset}", crate::render::sanitize_str(title))?;
    let width = rows.iter().map(|r| r.key.len()).max().unwrap_or(0);
    for row in rows.iter().filter(|r| !r.value.is_empty()) {
        let value = crate::render::sanitize_str(&row.value);
        let color = match (opts.colors, row.tone) {
            (false, _) | (_, Tone::Plain) => "",
            (_, Tone::Red) => "\x1b[31m",
            (_, Tone::Yellow) => "\x1b[33m",
            (_, Tone::Green) => "\x1b[32m",
        };
        let end = if color.is_empty() { "" } else { reset };
        writeln!(
            out,
            "  {dim}{:<width$}{reset}  {color}{value}{end}",
            row.key
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_content_and_name() {
        let pem = b"-----BEGIN CERTIFICATE-----\nMIIB\n";
        assert!(wants(Some("x.txt"), pem));
        assert!(wants(None, pem));
        assert!(wants(Some("cert"), pem));
        // Source code with an embedded certificate keeps its highlighting.
        assert!(!wants(Some("tls_test.go"), pem));
        // Not at the start of a line.
        assert!(!wants(
            Some("x.txt"),
            b"const C = \"-----BEGIN CERTIFICATE-----\"\n"
        ));
        assert!(wants(
            Some("key.pem"),
            b"-----BEGIN PGP PRIVATE KEY BLOCK-----\n"
        ));
        // PGP public keys stay text.
        assert!(!wants(
            Some("key.pem"),
            b"-----BEGIN PGP PUBLIC KEY BLOCK-----\n"
        ));
        // DER by extension only.
        assert!(wants(Some("a.der"), &[0x30, 0x82, 0x03, 0x00]));
        assert!(!wants(Some("a.bin"), &[0x30, 0x82, 0x03, 0x00]));
    }

    #[test]
    fn hashed_hosts() {
        assert_eq!(hosts("|1|abc|def"), "(hashed)");
        assert_eq!(hosts("@revoked a.test,|1|x|y"), "@revoked a.test,(hashed)");
    }

    #[test]
    fn private_block_without_end_is_hidden() {
        let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIEsecretsecret\n";
        let opts = Options {
            colors: false,
            utf8: true,
            tab_width: 4,
            now: 0,
        };
        let mut out = Vec::new();
        assert!(render(&mut out, text.as_bytes(), Some("k.pem"), &opts).unwrap());
        let out = String::from_utf8(out).unwrap();
        assert!(!out.contains("MIIEsecret"), "{out}");
        assert!(out.contains(HIDDEN_NOTE));
    }

    /// What `render_cert` prints in `auto`: the view, or the redacted text fallback.
    fn auto_view(data: &[u8]) -> String {
        let opts = Options {
            colors: false,
            utf8: true,
            tab_width: 4,
            now: 0,
        };
        let mut out = Vec::new();
        if !render(&mut out, data, Some("x.pem"), &opts).unwrap() {
            use std::io::Read;
            out.clear();
            redact::Redacting::new(data).read_to_end(&mut out).unwrap();
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    /// Many malformed arrangements of a private key: no line of its body may show.
    #[test]
    fn malformed_private_keys_never_show_their_body() {
        let key = include_str!("../../tests/fixtures/test-keys/rsa-pkcs8.key");
        let cert = include_str!("../../tests/fixtures/test-keys/leaf.pem");
        let body: Vec<&str> = key.lines().filter(|l| !l.starts_with("-----")).collect();
        assert!(body.len() > 5);
        let labels = [
            "PRIVATE KEY",
            "ENCRYPTED PRIVATE KEY",
            "OPENSSH PRIVATE KEY",
            "RSA PRIVATE KEY",
            "private key",
            "  PRIVATE   KEY ",
        ];
        let cert_head: String = cert.lines().take(4).map(|l| format!("{l}\n")).collect();
        let contexts: [(&str, &str); 6] = [
            ("", ""),
            (cert, ""),
            (&cert_head, ""),
            ("notes\n-----END PRIVATE KEY-----\n", "more notes\n"),
            (
                "-----BEGIN CERTIFICATE-----\n",
                "-----END CERTIFICATE-----\n",
            ),
            ("", cert),
        ];
        let junk = [
            "",
            "Proc-Type: 4,ENCRYPTED\n",
            "# comment\n\n",
            "-----BEGIN CERTIFICATE-----\n",
            "-----END CERTIFICATE-----\n",
        ];
        let mut cases = 0;
        for label in labels {
            for (before, after) in contexts {
                for junk in junk {
                    for end in [
                        format!("-----END {label}-----\n"),
                        String::new(),
                        "-----END SOMETHING ELSE-----\n".to_string(),
                        format!("-----end {}-----\n", label.to_lowercase()),
                    ] {
                        for indent in ["", "  "] {
                            let mut text = format!("{before}{indent}-----BEGIN {label}-----  \n");
                            for (idx, line) in body.iter().enumerate() {
                                if idx == body.len() / 2 {
                                    text.push_str(junk);
                                }
                                text.push_str(line);
                                text.push('\n');
                            }
                            text.push_str(&end);
                            text.push_str(after);
                            for crlf in [false, true] {
                                let data = if crlf {
                                    text.replace('\n', "\r\n")
                                } else {
                                    text.clone()
                                };
                                for truncate in [data.len(), data.len() * 2 / 3] {
                                    let data = &data.as_bytes()[..truncate];
                                    let out = auto_view(data);
                                    for line in &body {
                                        // A cut-off file may end inside a line.
                                        let probe = &line[..line.len().min(24)];
                                        assert!(
                                            !out.contains(probe),
                                            "body shown for {label:?} / {before:?} / {junk:?} / {end:?}:\n{out}"
                                        );
                                    }
                                    cases += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(cases > 1000);
    }

    #[test]
    fn nested_key_in_unfinished_certificate_is_hidden() {
        let key = include_str!("../../tests/fixtures/test-keys/ed25519-pkcs8.key");
        let cert = include_str!("../../tests/fixtures/test-keys/leaf.pem");
        let data = format!("{cert}-----BEGIN CERTIFICATE-----\n{key}");
        let out = auto_view(data.as_bytes());
        assert!(
            !out.contains("MC4CAQAwBQYDK2VwBCIEIOQyHQInCcsZcOQCbYxMwib0c2TPTBdWMmvk"),
            "{out}"
        );
        assert!(out.contains("Private key"), "{out}");
    }

    #[test]
    fn stray_invalid_byte_keeps_the_view() {
        let key = include_str!("../../tests/fixtures/test-keys/ed25519-pkcs8.key");
        let mut data = key.as_bytes().to_vec();
        data.extend_from_slice(b"# notes\n\xff\n");
        let out = auto_view(&data);
        assert!(out.contains("Private key"), "{out}");
        assert!(!out.contains("MC4CAQAw"), "{out}");
    }
}
