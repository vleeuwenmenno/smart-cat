//! Certificate and key view through the binary.
//!
//! The files in `tests/fixtures/test-keys/` are TEST KEYS, generated with openssl and
//! ssh-keygen for this suite only. They protect nothing; never use them anywhere else.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCAT: &str = env!("CARGO_BIN_EXE_scat");

fn keys_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-keys")
}

fn run_with(args: &[&str], config: Option<&Path>) -> Output {
    let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let mut cmd = Command::new(SCAT);
    cmd.args(args)
        .current_dir(keys_dir())
        .env_remove("SCAT_CONFIG")
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .env("COLUMNS", "100")
        .env("LC_ALL", "C.UTF-8");
    if let Some(config) = config {
        cmd.env("SCAT_CONFIG", config);
    }
    cmd.output().unwrap()
}

fn run(args: &[&str]) -> Output {
    run_with(args, None)
}

/// Rendered output without colours.
fn view(file: &str) -> String {
    let out = run(&["-f", "--color=never", file]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

fn read(file: &str) -> Vec<u8> {
    std::fs::read(keys_dir().join(file)).unwrap()
}

/// The base64 body lines of every PEM block in a file.
fn body_lines(file: &str) -> Vec<String> {
    let text = String::from_utf8(read(file)).unwrap();
    text.lines()
        .filter(|l| !l.starts_with("-----") && !l.contains(':') && l.len() > 8)
        .map(str::to_string)
        .collect()
}

const PRIVATE_KEYS: &[&str] = &[
    "id_ed25519",
    "id_ed25519_encrypted",
    "rsa-pkcs1.key",
    "rsa-pkcs8.key",
    "ec-sec1.key",
    "ed25519-pkcs8.key",
    "encrypted-pkcs8.key",
    "rsa-legacy-encrypted.key",
];

#[test]
fn piped_output_is_the_file() {
    for file in PRIVATE_KEYS.iter().chain(&[
        "bundle.pem",
        "leaf.der",
        "authorized_keys",
        "known_hosts",
        "mixed.txt",
    ]) {
        let out = run(&[file]);
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(out.stdout, read(file), "{file}");
        let out = run(&["--raw", file]);
        assert_eq!(out.stdout, read(file), "--raw {file}");
    }
}

#[test]
fn private_keys_never_show_their_body() {
    for file in PRIVATE_KEYS {
        let shown = view(file);
        assert!(
            shown.contains("private key material hidden; use --raw or a pipe to print it"),
            "{file}: {shown}"
        );
        let lines = body_lines(file);
        assert!(!lines.is_empty(), "{file}");
        for line in lines {
            assert!(!shown.contains(&line), "{file} leaked {line}");
            // Not even a fragment of a body line.
            let part = &line[..line.len().min(16)];
            assert!(!shown.contains(part), "{file} leaked part of {line}");
        }
    }
}

#[test]
fn private_key_details() {
    let rsa = view("rsa-pkcs1.key");
    assert!(rsa.contains("RSA 2048 bits"), "{rsa}");
    assert!(rsa.contains("encrypted    no"), "{rsa}");
    // Same as `ssh-keygen -lf rsa-pkcs1.key`.
    assert!(rsa.contains("SHA256:soHmsJhjLWrnEA2TRTMCIQKQ14S4YZPblBAYPCQQTlg"));

    let ssh = view("id_ed25519");
    assert!(ssh.contains("SHA256:ye6cAp7aB4kY7igETRN0NKKnC3i1+nVWavNBs86ro9M"));
    assert!(ssh.contains("test@scat"));

    let enc = view("id_ed25519_encrypted");
    assert!(
        enc.contains("encrypted    yes, aes256-ctr (bcrypt)"),
        "{enc}"
    );
    assert!(enc.contains("SHA256:xLFbSobiOoDQf3qvBo8mO03ca52nPFP8g8Bfx5NBQEE"));

    let ec = view("ec-sec1.key");
    assert!(ec.contains("EC P-256"), "{ec}");
    assert!(ec.contains("SHA256:a0CKtyJ3Np1jt6flsgIgG639uVAlURUYgovcep3mOkg"));

    assert!(view("encrypted-pkcs8.key").contains("PBES2 (PBKDF2, AES-256-CBC)"));
    assert!(view("rsa-legacy-encrypted.key").contains("yes, AES-256-CBC"));
}

#[test]
fn explicit_text_renderers_print_private_keys_like_cat() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cert-config");
    std::fs::create_dir_all(&dir).unwrap();
    for renderer in ["code", "text", "raw"] {
        let config = dir.join(format!("{renderer}.toml"));
        std::fs::write(
            &config,
            format!("[defaults]\nline_numbers = false\n\n[[rule]]\nmatch = {{ ext = \"key\" }}\nrenderer = \"{renderer}\"\n"),
        )
        .unwrap();
        let out = run_with(&["-f", "--color=never", "rsa-pkcs1.key"], Some(&config));
        assert_eq!(out.stdout, read("rsa-pkcs1.key"), "renderer = {renderer}");
    }
}

#[test]
fn bundle_lists_each_certificate_and_the_chain() {
    let shown = view("bundle.pem");
    for title in [
        "Certificate 1 of 3",
        "Certificate 2 of 3",
        "Certificate 3 of 3",
    ] {
        assert!(shown.contains(title), "{shown}");
    }
    assert!(shown.contains(
        "chain: #1 leaf.scat.test → #2 scat Test Intermediate → #3 scat Test Root CA (self-signed)"
    ));
    assert!(shown.contains("CA           yes, path length 0"), "{shown}");
    assert!(!shown.contains("-----BEGIN"));
}

#[test]
fn certificate_fields() {
    let shown = view("leaf.pem");
    let expected = [
        "subject         CN=leaf.scat.test, O=scat tests",
        "issuer          CN=scat Test Intermediate, O=scat tests",
        "serial          2A:2B:2C",
        "not before      2025-06-01 00:00:00 UTC",
        "not after       2035-06-01 00:00:00 UTC (expires in",
        "SANs            DNS leaf.scat.test",
        "                DNS *.scat.test",
        "                IP 192.0.2.7",
        "                IP 2001:db8::1",
        "                email admin@scat.test",
        "key             RSA 2048 bits",
        "signature       ecdsa-with-SHA256",
        "CA              no",
        "key usage       digital signature, key encipherment (critical)",
        "extended usage  server auth, client auth",
        "SHA-256         9E:70:7E:3A:27:99:D9:03:2F:23:2D:6F:63:81:75:32:87:46:B8:1D:CE:95:4D:59:33:A3:6E:22:98:7A:4F:76",
        "SHA-1           5F:2D:D0:6E:98:E6:39:20:4D:57:09:5E:D8:2C:F8:F9:7F:91:C5:8D",
        "self-signed     no",
    ];
    for line in expected {
        assert!(shown.contains(line), "missing {line:?} in\n{shown}");
    }
    // DER gives the same summary.
    assert_eq!(view("leaf.der"), shown);
    assert!(view("root.pem").contains("self-signed  yes"));
}

#[test]
fn validity_is_coloured() {
    let out = run(&["-f", "--color=always", "leaf.pem"]);
    let shown = String::from_utf8(out.stdout).unwrap();
    // Valid until 2035: green.
    assert!(
        shown.contains("\x1b[32m2035-06-01 00:00:00 UTC (expires in"),
        "{shown:?}"
    );
}

#[test]
fn request_shows_subject_sans_and_key() {
    let shown = view("leaf.csr");
    assert!(shown.starts_with("Certificate request\n"), "{shown}");
    assert!(shown.contains("CN=leaf.scat.test"));
    assert!(shown.contains("DNS *.scat.test"));
    assert!(shown.contains("RSA 2048 bits"));
}

#[test]
fn ssh_fingerprints_match_ssh_keygen() {
    // Expected values come from `ssh-keygen -lf FILE`.
    let shown = view("authorized_keys");
    let lines: Vec<&str> = shown.lines().collect();
    assert_eq!(lines[0], "SSH public keys (3 keys)");
    assert_eq!(
        lines[1],
        "  ED25519  256  SHA256:ye6cAp7aB4kY7igETRN0NKKnC3i1+nVWavNBs86ro9M  test@scat"
    );
    assert!(lines[2].starts_with(
        "  ECDSA    256  SHA256:2LCX1hZb3FZXI7O3xtOFy+jIfCa8ArKLKEfcJsH9lM0  ecdsa@scat  options: from="
    ));
    assert_eq!(
        lines[3],
        "  RSA     2048  SHA256:KY1kcXW2maE9B4yk6orHV988i06PMXz+At1Bk1R6HnA  rsa@scat"
    );

    let hosts = view("known_hosts");
    assert!(
        hosts.contains("SHA256:ye6cAp7aB4kY7igETRN0NKKnC3i1+nVWavNBs86ro9M  github.test,192.0.2.1")
    );
    assert!(hosts.contains("SHA256:2LCX1hZb3FZXI7O3xtOFy+jIfCa8ArKLKEfcJsH9lM0  (hashed)"));
    assert!(!hosts.contains("|1|"));

    let single = view("id_ed25519.pub");
    assert!(single.contains("fingerprint  SHA256:ye6cAp7aB4kY7igETRN0NKKnC3i1+nVWavNBs86ro9M"));
    assert!(single.contains("comment      test@scat"));
}

#[test]
fn mixed_file_keeps_the_text_around_the_summary() {
    let shown = view("mixed.txt");
    assert!(
        shown.starts_with("TEST KEYS for the scat test suite."),
        "{shown}"
    );
    assert!(shown.contains("\nCertificate\n  subject"));
    assert!(
        shown.ends_with("self-signed     no\n\ntrailing note\n"),
        "{shown}"
    );
}

#[test]
fn private_key_inside_other_text_is_hidden() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cert-mixed");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notes.txt");
    let key = String::from_utf8(read("ec-sec1.key")).unwrap();
    std::fs::write(&file, format!("server key:\n{key}end of notes\n")).unwrap();
    let out = run(&["-f", "--color=never", file.to_str().unwrap()]);
    let shown = String::from_utf8(out.stdout).unwrap();
    assert!(shown.starts_with("server key:\n\nPrivate key\n"), "{shown}");
    assert!(shown.ends_with("\n\nend of notes\n"), "{shown}");
    for line in body_lines("ec-sec1.key") {
        assert!(!shown.contains(&line), "leaked {line}");
    }
}

#[test]
fn unparsable_pem_falls_back_to_text() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cert-broken");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("broken.pem");
    let content = "-----BEGIN CERTIFICATE-----\nbm90IGEgY2VydA==\n-----END CERTIFICATE-----\n";
    std::fs::write(&file, content).unwrap();
    let out = run(&["-f", "--color=never", "-p", file.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), content);
}
