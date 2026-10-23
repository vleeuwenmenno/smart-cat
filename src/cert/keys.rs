//! Describing public and private keys: algorithm, size or curve, OpenSSH fingerprint.
//! Private keys are only inspected for what is public (type, size, public half);
//! nothing returned here contains private key material.

use sha2::{Digest, Sha256};

use super::der::{self, BIT_STRING, INTEGER, OCTET_STRING, OID, SEQUENCE, Wire};

/// What can be told about a key without its secret.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyInfo {
    /// "RSA", "EC", "Ed25519", ... (or a dotted OID).
    pub algorithm: String,
    pub bits: Option<u32>,
    /// Curve name for EC keys, e.g. "P-256".
    pub curve: Option<&'static str>,
    /// OpenSSH SHA256 fingerprint of the public key, when it is known.
    pub fingerprint: Option<String>,
}

impl KeyInfo {
    /// "RSA 2048 bits", "EC P-256 (256 bits)", "Ed25519".
    pub fn summary(&self) -> String {
        match (self.curve, self.bits) {
            (Some(curve), Some(bits)) => format!("{} {curve} ({bits} bits)", self.algorithm),
            (Some(curve), None) => format!("{} {curve}", self.algorithm),
            (None, Some(bits)) => format!("{} {bits} bits", self.algorithm),
            (None, None) => self.algorithm.clone(),
        }
    }
}

// DER contents of the OIDs scat names.
const RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const RSA_PSS: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0a];
const EC: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const DSA: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x38, 0x04, 0x01];
const ED25519: &[u8] = &[0x2b, 0x65, 0x70];
const ED448: &[u8] = &[0x2b, 0x65, 0x71];
const X25519: &[u8] = &[0x2b, 0x65, 0x6e];
const X448: &[u8] = &[0x2b, 0x65, 0x6f];
const PBES2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0d];
const PBKDF2: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x05, 0x0c];
const SCRYPT: &[u8] = &[0x2b, 0x06, 0x01, 0x04, 0x01, 0xda, 0x47, 0x04, 0x0b];

/// Named curves: OID, display name, bits, OpenSSH curve name.
const CURVES: &[(&[u8], &str, u32, Option<&str>)] = &[
    (
        &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07],
        "P-256",
        256,
        Some("nistp256"),
    ),
    (
        &[0x2b, 0x81, 0x04, 0x00, 0x22],
        "P-384",
        384,
        Some("nistp384"),
    ),
    (
        &[0x2b, 0x81, 0x04, 0x00, 0x23],
        "P-521",
        521,
        Some("nistp521"),
    ),
    (&[0x2b, 0x81, 0x04, 0x00, 0x0a], "secp256k1", 256, None),
    (
        &[0x2b, 0x24, 0x03, 0x03, 0x02, 0x08, 0x01, 0x01, 0x07],
        "brainpoolP256r1",
        256,
        None,
    ),
    (
        &[0x2b, 0x24, 0x03, 0x03, 0x02, 0x08, 0x01, 0x01, 0x0b],
        "brainpoolP384r1",
        384,
        None,
    ),
    (
        &[0x2b, 0x24, 0x03, 0x03, 0x02, 0x08, 0x01, 0x01, 0x0d],
        "brainpoolP512r1",
        512,
        None,
    ),
];

fn curve(oid: &[u8]) -> Option<(&'static str, u32, Option<&'static str>)> {
    CURVES
        .iter()
        .find(|(o, ..)| *o == oid)
        .map(|&(_, name, bits, ssh)| (name, bits, ssh))
}

/// OpenSSH `SHA256:...` fingerprint of a public key blob.
pub fn ssh_fingerprint(blob: &[u8]) -> String {
    format!(
        "SHA256:{}",
        der::base64_encode_unpadded(&Sha256::digest(blob))
    )
}

fn rsa_blob(n: &[u8], e: &[u8]) -> Vec<u8> {
    let mut blob = Vec::new();
    der::put_string(&mut blob, b"ssh-rsa");
    der::put_string(&mut blob, e);
    der::put_string(&mut blob, n);
    blob
}

fn ec_blob(ssh_curve: &str, point: &[u8]) -> Vec<u8> {
    let mut blob = Vec::new();
    der::put_string(&mut blob, format!("ecdsa-sha2-{ssh_curve}").as_bytes());
    der::put_string(&mut blob, ssh_curve.as_bytes());
    der::put_string(&mut blob, point);
    blob
}

fn ed25519_blob(key: &[u8]) -> Vec<u8> {
    let mut blob = Vec::new();
    der::put_string(&mut blob, b"ssh-ed25519");
    der::put_string(&mut blob, key);
    blob
}

/// An `AlgorithmIdentifier`: the algorithm OID and its parameters element.
fn algorithm_id(value: &[u8]) -> Option<(&[u8], Option<der::Tlv<'_>>)> {
    let (oid, rest) = der::expect(value, OID)?;
    Some((oid, der::read(rest).map(|(tlv, _)| tlv)))
}

fn simple_algorithm(oid: &[u8]) -> Option<(&'static str, Option<u32>)> {
    Some(match oid {
        ED25519 => ("Ed25519", Some(256)),
        ED448 => ("Ed448", Some(456)),
        X25519 => ("X25519", Some(256)),
        X448 => ("X448", Some(448)),
        _ => return None,
    })
}

/// A PKCS#1 `RSAPublicKey` (`SEQUENCE { n, e }`).
pub fn describe_rsa_public(der_bytes: &[u8]) -> Option<KeyInfo> {
    let (seq, _) = der::expect(der_bytes, SEQUENCE)?;
    let (n, rest) = der::expect(seq, INTEGER)?;
    let (e, _) = der::expect(rest, INTEGER)?;
    Some(KeyInfo {
        algorithm: "RSA".into(),
        bits: Some(der::int_bits(n)),
        curve: None,
        fingerprint: Some(ssh_fingerprint(&rsa_blob(n, e))),
    })
}

/// A `SubjectPublicKeyInfo`, as in certificates, CSRs and `PUBLIC KEY` blocks.
pub fn describe_spki(der_bytes: &[u8]) -> Option<KeyInfo> {
    let (seq, _) = der::expect(der_bytes, SEQUENCE)?;
    let (alg, rest) = der::expect(seq, SEQUENCE)?;
    let (key, _) = der::expect(rest, BIT_STRING)?;
    let key = der::bit_string(key)?;
    let (oid, params) = algorithm_id(alg)?;
    match oid {
        RSA | RSA_PSS => {
            let mut info = describe_rsa_public(key)?;
            if oid == RSA_PSS {
                info.algorithm = "RSA-PSS".into();
            }
            Some(info)
        }
        EC => {
            let named = params.filter(|p| p.tag == OID).and_then(|p| curve(p.value));
            Some(KeyInfo {
                algorithm: "EC".into(),
                bits: named.map(|(_, bits, _)| bits),
                curve: named.map(|(name, ..)| name),
                fingerprint: named
                    .and_then(|(_, _, ssh)| ssh)
                    .map(|ssh| ssh_fingerprint(&ec_blob(ssh, key))),
            })
        }
        DSA => {
            let p_bits = params
                .filter(|p| p.tag == SEQUENCE)
                .and_then(|p| der::expect(p.value, INTEGER))
                .map(|(p, _)| der::int_bits(p));
            Some(KeyInfo {
                algorithm: "DSA".into(),
                bits: p_bits,
                ..KeyInfo::default()
            })
        }
        _ => {
            let (name, bits) = simple_algorithm(oid).unwrap_or(("", None));
            Some(KeyInfo {
                algorithm: if name.is_empty() {
                    der::oid_string(oid)
                } else {
                    name.into()
                },
                bits,
                curve: None,
                fingerprint: (oid == ED25519 && key.len() == 32)
                    .then(|| ssh_fingerprint(&ed25519_blob(key))),
            })
        }
    }
}

/// A private key as far as it can be described without printing it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivateInfo {
    pub key: KeyInfo,
    /// "PKCS#8", "PKCS#1", "SEC1", "OpenSSH", ...
    pub format: &'static str,
    /// `None` when not encrypted, otherwise the scheme when known.
    pub encryption: Option<String>,
    /// OpenSSH keys carry a comment (only readable when not encrypted).
    pub comment: Option<String>,
}

/// A PKCS#1 `RSAPrivateKey`: `SEQUENCE { version, n, e, d, ... }`.
fn rsa_private(der_bytes: &[u8]) -> Option<KeyInfo> {
    let (seq, _) = der::expect(der_bytes, SEQUENCE)?;
    let (_, rest) = der::expect(seq, INTEGER)?;
    let (n, rest) = der::expect(rest, INTEGER)?;
    let (e, _) = der::expect(rest, INTEGER)?;
    Some(KeyInfo {
        algorithm: "RSA".into(),
        bits: Some(der::int_bits(n)),
        curve: None,
        fingerprint: Some(ssh_fingerprint(&rsa_blob(n, e))),
    })
}

/// A SEC1 `ECPrivateKey`: `SEQUENCE { 1, d, [0] curve OPTIONAL, [1] point OPTIONAL }`.
/// `curve_oid` comes from the PKCS#8 wrapper when the inner key has none.
fn ec_private(der_bytes: &[u8], curve_oid: Option<&[u8]>) -> Option<KeyInfo> {
    let (seq, _) = der::expect(der_bytes, SEQUENCE)?;
    let items = der::items(seq)?;
    let mut oid = curve_oid;
    let mut point = None;
    for item in items.iter().skip(2) {
        match item.tag {
            0xa0 => oid = der::expect(item.value, OID).map(|(o, _)| o).or(oid),
            0xa1 => {
                point = der::expect(item.value, BIT_STRING).and_then(|(b, _)| der::bit_string(b))
            }
            _ => {}
        }
    }
    let named = oid.and_then(curve);
    Some(KeyInfo {
        algorithm: "EC".into(),
        bits: named.map(|(_, bits, _)| bits),
        curve: named.map(|(name, ..)| name),
        fingerprint: match (named.and_then(|(_, _, ssh)| ssh), point) {
            (Some(ssh), Some(point)) => Some(ssh_fingerprint(&ec_blob(ssh, point))),
            _ => None,
        },
    })
}

/// A PKCS#8 `PrivateKeyInfo` / `OneAsymmetricKey`.
fn pkcs8(der_bytes: &[u8]) -> Option<KeyInfo> {
    let (seq, _) = der::expect(der_bytes, SEQUENCE)?;
    let (_, rest) = der::expect(seq, INTEGER)?;
    let (alg, rest) = der::expect(rest, SEQUENCE)?;
    let (inner, rest) = der::expect(rest, OCTET_STRING)?;
    let (oid, params) = algorithm_id(alg)?;
    match oid {
        RSA | RSA_PSS => {
            let mut info = rsa_private(inner)?;
            if oid == RSA_PSS {
                info.algorithm = "RSA-PSS".into();
            }
            Some(info)
        }
        EC => ec_private(inner, params.filter(|p| p.tag == OID).map(|p| p.value)),
        DSA => Some(KeyInfo {
            algorithm: "DSA".into(),
            bits: params
                .filter(|p| p.tag == SEQUENCE)
                .and_then(|p| der::expect(p.value, INTEGER))
                .map(|(p, _)| der::int_bits(p)),
            ..KeyInfo::default()
        }),
        _ => {
            let (name, bits) = simple_algorithm(oid).unwrap_or(("", None));
            // OneAsymmetricKey (v2) may carry the public key as [1].
            let public = der::items(rest)
                .unwrap_or_default()
                .into_iter()
                .find(|t| t.tag == 0x81)
                .and_then(|t| der::bit_string(t.value));
            Some(KeyInfo {
                algorithm: if name.is_empty() {
                    der::oid_string(oid)
                } else {
                    name.into()
                },
                bits,
                curve: None,
                fingerprint: public
                    .filter(|k| oid == ED25519 && k.len() == 32)
                    .map(|k| ssh_fingerprint(&ed25519_blob(k))),
            })
        }
    }
}

/// `EncryptedPrivateKeyInfo`: only the scheme is readable.
fn encrypted_pkcs8_scheme(der_bytes: &[u8]) -> Option<String> {
    let (seq, _) = der::expect(der_bytes, SEQUENCE)?;
    let (alg, _) = der::expect(seq, SEQUENCE)?;
    let (oid, params) = algorithm_id(alg)?;
    if oid != PBES2 {
        return Some(der::oid_string(oid));
    }
    let params = params.filter(|p| p.tag == SEQUENCE)?;
    let (kdf, rest) = der::expect(params.value, SEQUENCE)?;
    let (cipher, _) = der::expect(rest, SEQUENCE)?;
    let kdf = match algorithm_id(kdf)?.0 {
        PBKDF2 => "PBKDF2".to_string(),
        SCRYPT => "scrypt".to_string(),
        other => der::oid_string(other),
    };
    let cipher_oid = algorithm_id(cipher)?.0;
    let cipher = match cipher_oid {
        [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x02] => "AES-128-CBC".to_string(),
        [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x16] => "AES-192-CBC".to_string(),
        [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x01, 0x2a] => "AES-256-CBC".to_string(),
        [0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x03, 0x07] => "DES-EDE3-CBC".to_string(),
        other => der::oid_string(other),
    };
    Some(format!("PBES2 ({kdf}, {cipher})"))
}

/// Describes a private key PEM block (or DER for `PRIVATE KEY`). `headers` are the
/// RFC 1421 headers of a legacy PEM block (`Proc-Type`, `DEK-Info`). Returns a
/// description even when the body cannot be parsed: the block is hidden either way.
pub fn describe_private(
    label: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> PrivateInfo {
    let legacy_cipher = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("DEK-Info"))
        .map(|(_, v)| v.split(',').next().unwrap_or(v).trim().to_string());
    let legacy_encrypted = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("Proc-Type") && v.contains("ENCRYPTED"));
    let unknown = |algorithm: &str, format| PrivateInfo {
        key: KeyInfo {
            algorithm: algorithm.into(),
            ..KeyInfo::default()
        },
        format,
        encryption: None,
        comment: None,
    };
    let legacy = |algorithm: &str, parse: &dyn Fn(&[u8]) -> Option<KeyInfo>, format| {
        if legacy_encrypted {
            let mut info = unknown(algorithm, format);
            info.encryption = Some(legacy_cipher.clone().unwrap_or_else(|| "yes".into()));
            return info;
        }
        let mut info = unknown(algorithm, format);
        if let Some(key) = body.and_then(parse) {
            info.key = key;
        }
        info
    };
    match label {
        "RSA PRIVATE KEY" => legacy("RSA", &rsa_private, "PKCS#1"),
        "EC PRIVATE KEY" => legacy("EC", &|b| ec_private(b, None), "SEC1"),
        "DSA PRIVATE KEY" => legacy(
            "DSA",
            &|b| {
                let (seq, _) = der::expect(b, SEQUENCE)?;
                let (_, rest) = der::expect(seq, INTEGER)?;
                let (p, _) = der::expect(rest, INTEGER)?;
                Some(KeyInfo {
                    algorithm: "DSA".into(),
                    bits: Some(der::int_bits(p)),
                    ..KeyInfo::default()
                })
            },
            "DSA",
        ),
        "PRIVATE KEY" => {
            let mut info = unknown("unknown", "PKCS#8");
            if let Some(key) = body.and_then(pkcs8) {
                info.key = key;
            }
            info
        }
        "ENCRYPTED PRIVATE KEY" => {
            let mut info = unknown("unknown", "PKCS#8");
            info.encryption = Some(
                body.and_then(encrypted_pkcs8_scheme)
                    .unwrap_or_else(|| "yes".into()),
            );
            info
        }
        "OPENSSH PRIVATE KEY" => body
            .and_then(openssh_private)
            .unwrap_or_else(|| unknown("unknown", "OpenSSH")),
        _ => unknown("unknown", "unknown"),
    }
}

/// Describes an OpenSSH public key blob: ssh-keygen's type label and size.
pub fn describe_ssh_blob(blob: &[u8]) -> Option<KeyInfo> {
    let mut wire = Wire(blob);
    let full_type = wire.str()?;
    let (base, cert) = match full_type.strip_suffix("-cert-v01@openssh.com") {
        Some(base) => (base, true),
        None => (full_type, false),
    };
    if cert {
        wire.string()?; // nonce
    }
    let (label, bits) = match base {
        "ssh-rsa" => {
            let _e = wire.string()?;
            ("RSA", der::int_bits(wire.string()?))
        }
        "ssh-dss" => ("DSA", der::int_bits(wire.string()?)),
        "ssh-ed25519" => ("ED25519", 256),
        "sk-ssh-ed25519@openssh.com" => ("ED25519-SK", 256),
        "ecdsa-sha2-nistp256" => ("ECDSA", 256),
        "ecdsa-sha2-nistp384" => ("ECDSA", 384),
        "ecdsa-sha2-nistp521" => ("ECDSA", 521),
        "sk-ecdsa-sha2-nistp256@openssh.com" => ("ECDSA-SK", 256),
        _ => return None,
    };
    let curve = match base {
        "ecdsa-sha2-nistp256" | "sk-ecdsa-sha2-nistp256@openssh.com" => Some("P-256"),
        "ecdsa-sha2-nistp384" => Some("P-384"),
        "ecdsa-sha2-nistp521" => Some("P-521"),
        _ => None,
    };
    Some(KeyInfo {
        algorithm: if cert {
            format!("{label}-CERT")
        } else {
            label.to_string()
        },
        bits: Some(bits),
        curve,
        fingerprint: Some(ssh_fingerprint(blob)),
    })
}

/// Number of private fields after the key type in an unencrypted OpenSSH key, as
/// (strings, then a flags byte, then strings). Only used to reach the comment.
fn openssh_private_layout(key_type: &str) -> Option<(usize, bool, usize)> {
    Some(match key_type {
        "ssh-ed25519" => (2, false, 0),
        "ssh-rsa" => (6, false, 0),
        "ssh-dss" => (5, false, 0),
        t if t.starts_with("ecdsa-sha2-") => (3, false, 0),
        "sk-ssh-ed25519@openssh.com" => (2, true, 2),
        "sk-ecdsa-sha2-nistp256@openssh.com" => (3, true, 2),
        _ => return None,
    })
}

/// An `OPENSSH PRIVATE KEY` body. The public key is stored in the clear, so type, size
/// and fingerprint are known even for encrypted keys; the comment is not.
fn openssh_private(body: &[u8]) -> Option<PrivateInfo> {
    let rest = body.strip_prefix(b"openssh-key-v1\0")?;
    let mut wire = Wire(rest);
    let cipher = wire.str()?;
    let kdf = wire.str()?;
    wire.string()?; // kdf options
    let count = wire.u32()?;
    if count == 0 {
        return None;
    }
    let public = wire.string()?;
    for _ in 1..count {
        wire.string()?;
    }
    let key = describe_ssh_blob(public)?;
    let encryption = (cipher != "none").then(|| format!("{cipher} ({kdf})"));
    let comment = if encryption.is_none() {
        wire.string().and_then(|private| {
            let mut p = Wire(private);
            p.u32()?;
            p.u32()?;
            let key_type = p.str()?;
            let (before, flags, after) = openssh_private_layout(key_type)?;
            for _ in 0..before {
                p.string()?;
            }
            if flags {
                p.byte()?;
            }
            for _ in 0..after {
                p.string()?;
            }
            p.str().map(str::to_string)
        })
    } else {
        None
    };
    Some(PrivateInfo {
        key,
        format: "OpenSSH",
        encryption,
        comment: comment.filter(|c| !c.is_empty()),
    })
}

/// One parsed line of an OpenSSH public key file, `authorized_keys` or `known_hosts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshLine {
    /// Text before the key type: host patterns or options (and markers).
    pub prefix: Option<String>,
    pub key: KeyInfo,
    pub comment: Option<String>,
}

const SSH_TYPES: &[&str] = &[
    "ssh-rsa",
    "ssh-dss",
    "ssh-ed25519",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// Whether a word is an OpenSSH key type (certificates included).
pub fn is_ssh_type(word: &str) -> bool {
    let base = word.strip_suffix("-cert-v01@openssh.com").unwrap_or(word);
    SSH_TYPES.contains(&base)
}

/// Splits on spaces and tabs, keeping double-quoted parts (authorized_keys options)
/// together.
fn words(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => {
                quoted = !quoted;
                start.get_or_insert(i);
            }
            ' ' | '\t' if !quoted => {
                if let Some(s) = start.take() {
                    out.push((s, &line[s..i]));
                }
            }
            _ => {
                start.get_or_insert(i);
            }
        }
    }
    if let Some(s) = start {
        out.push((s, &line[s..]));
    }
    out
}

/// Parses one line: `[prefix] TYPE BASE64 [comment]`. Blank and `#` lines give `None`.
pub fn parse_ssh_line(line: &str) -> Option<SshLine> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let words = words(line);
    let idx = words.iter().position(|(_, w)| is_ssh_type(w))?;
    let (_, b64) = *words.get(idx + 1)?;
    let blob = der::base64_decode(b64.as_bytes())?;
    let key = describe_ssh_blob(&blob)?;
    let blob_type = Wire(&blob).str()?;
    if blob_type != words[idx].1 {
        return None;
    }
    let prefix = (idx > 0).then(|| line[..words[idx].0].trim_end().to_string());
    let comment = words
        .get(idx + 2)
        .map(|(pos, _)| line[*pos..].trim().to_string());
    Some(SshLine {
        prefix,
        key,
        comment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_options() {
        let w: Vec<&str> = words(r#"from="a b",no-pty ssh-ed25519 AAAA c d"#)
            .into_iter()
            .map(|(_, w)| w)
            .collect();
        assert_eq!(w, [r#"from="a b",no-pty"#, "ssh-ed25519", "AAAA", "c", "d"]);
    }

    #[test]
    fn rejects_mismatched_blob_type() {
        // An ed25519 blob announced as ssh-rsa is not a key line.
        let mut blob = Vec::new();
        der::put_string(&mut blob, b"ssh-ed25519");
        der::put_string(&mut blob, &[1u8; 32]);
        let b64 = der::base64_encode_unpadded(&blob);
        assert!(parse_ssh_line(&format!("ssh-rsa {b64}")).is_none());
        let ok = parse_ssh_line(&format!("ssh-ed25519 {b64} me@host")).unwrap();
        assert_eq!(ok.key.algorithm, "ED25519");
        assert_eq!(ok.comment.as_deref(), Some("me@host"));
        assert!(ok.prefix.is_none());
    }
}
