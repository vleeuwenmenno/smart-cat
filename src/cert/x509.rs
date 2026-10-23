//! Summaries of X.509 certificates and certificate requests (x509-parser).

use std::net::{Ipv4Addr, Ipv6Addr};

use sha1::Sha1;
use sha2::{Digest, Sha256};
use x509_parser::extensions::{GeneralName, ParsedExtension};
use x509_parser::oid_registry::Oid;
use x509_parser::prelude::{FromDer, X509Certificate, X509CertificationRequest, X509Name};

use super::der;
use super::keys;
use super::{Row, Summary, Tone};

/// A parsed certificate: its summary and the raw names that link a chain.
pub struct Cert {
    pub summary: Summary,
    pub subject: Vec<u8>,
    pub issuer: Vec<u8>,
    pub self_signed: bool,
    /// Short name for the chain line: the subject CN, or the whole subject.
    pub short_name: String,
}

/// Parses one DER certificate. Trailing bytes are not allowed.
pub fn certificate(der_bytes: &[u8], now: i64) -> Option<Cert> {
    let (rest, cert) = X509Certificate::from_der(der_bytes).ok()?;
    if !rest.is_empty() {
        return None;
    }
    let tbs = &cert.tbs_certificate;
    let mut rows = vec![
        Row::new("subject", name(&tbs.subject)),
        Row::new("issuer", name(&tbs.issuer)),
        Row::new("serial", serial(tbs.raw_serial())),
    ];
    let not_before = tbs.validity.not_before.timestamp();
    let not_after = tbs.validity.not_after.timestamp();
    rows.push(Row::new("not before", format_time(not_before)));
    let (tone, relative) = expiry(not_before, not_after, now);
    rows.push(Row::toned(
        "not after",
        format!("{} ({relative})", format_time(not_after)),
        tone,
    ));

    if let Ok(Some(san)) = cert.subject_alternative_name() {
        push_names(&mut rows, "SANs", &san.value.general_names);
    }
    rows.push(Row::new("key", key_summary(tbs.subject_pki.raw)));
    rows.push(Row::new(
        "signature",
        oid_name(&cert.signature_algorithm.algorithm),
    ));
    match cert.basic_constraints() {
        Ok(Some(bc)) => {
            let value = match (bc.value.ca, bc.value.path_len_constraint) {
                (true, Some(len)) => format!("yes, path length {len}"),
                (true, None) => "yes".into(),
                (false, _) => "no".into(),
            };
            rows.push(Row::new("CA", value));
        }
        _ => rows.push(Row::new("CA", "no (no basic constraints)".into())),
    }
    if let Ok(Some(ku)) = cert.key_usage() {
        let k = ku.value;
        let names = [
            (k.digital_signature(), "digital signature"),
            (k.non_repudiation(), "non-repudiation"),
            (k.key_encipherment(), "key encipherment"),
            (k.data_encipherment(), "data encipherment"),
            (k.key_agreement(), "key agreement"),
            (k.key_cert_sign(), "certificate signing"),
            (k.crl_sign(), "CRL signing"),
            (k.encipher_only(), "encipher only"),
            (k.decipher_only(), "decipher only"),
        ];
        let list: Vec<&str> = names
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, n)| *n)
            .collect();
        rows.push(Row::new(
            "key usage",
            critical(list.join(", "), ku.critical),
        ));
    }
    if let Ok(Some(eku)) = cert.extended_key_usage() {
        let e = eku.value;
        let mut list: Vec<String> = [
            (e.any, "any"),
            (e.server_auth, "server auth"),
            (e.client_auth, "client auth"),
            (e.code_signing, "code signing"),
            (e.email_protection, "email protection"),
            (e.time_stamping, "time stamping"),
            (e.ocsp_signing, "OCSP signing"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| n.to_string())
        .collect();
        list.extend(e.other.iter().map(oid_name));
        rows.push(Row::new(
            "extended usage",
            critical(list.join(", "), eku.critical),
        ));
    }
    rows.push(Row::new(
        "SHA-256",
        der::hex_colons(&Sha256::digest(der_bytes)),
    ));
    rows.push(Row::new("SHA-1", der::hex_colons(&Sha1::digest(der_bytes))));

    let self_signed = is_self_signed(&cert);
    rows.push(Row::new(
        "self-signed",
        if self_signed { "yes" } else { "no" }.into(),
    ));
    let short_name = tbs
        .subject
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| name(&tbs.subject));
    Some(Cert {
        summary: Summary {
            title: "Certificate".into(),
            rows,
            note: None,
        },
        subject: tbs.subject.as_raw().to_vec(),
        issuer: tbs.issuer.as_raw().to_vec(),
        self_signed,
        short_name,
    })
}

/// Parses a DER certificate request (PKCS#10).
pub fn request(der_bytes: &[u8]) -> Option<Summary> {
    let (rest, csr) = X509CertificationRequest::from_der(der_bytes).ok()?;
    if !rest.is_empty() {
        return None;
    }
    let info = &csr.certification_request_info;
    let mut rows = vec![Row::new("subject", name(&info.subject))];
    if let Some(extensions) = csr.requested_extensions() {
        for ext in extensions {
            if let ParsedExtension::SubjectAlternativeName(san) = ext {
                push_names(&mut rows, "SANs", &san.general_names);
            }
        }
    }
    rows.push(Row::new("key", key_summary(info.subject_pki.raw)));
    rows.push(Row::new(
        "signature",
        oid_name(&csr.signature_algorithm.algorithm),
    ));
    Some(Summary {
        title: "Certificate request".into(),
        rows,
        note: None,
    })
}

fn key_summary(spki: &[u8]) -> String {
    keys::describe_spki(spki).map_or_else(|| "unknown".into(), |k| k.summary())
}

fn name(name: &X509Name) -> String {
    let text = name.to_string();
    if text.is_empty() {
        "(empty)".into()
    } else {
        text
    }
}

fn critical(text: String, critical: bool) -> String {
    if critical {
        format!("{text} (critical)")
    } else {
        text
    }
}

fn oid_name(oid: &Oid) -> String {
    x509_parser::objects::oid2sn(oid, x509_parser::objects::oid_registry())
        .map(str::to_string)
        .unwrap_or_else(|_| oid.to_id_string())
}

/// The serial as hex without leading zero bytes, like `openssl x509 -serial`.
fn serial(raw: &[u8]) -> String {
    let start = raw
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(raw.len().saturating_sub(1));
    der::hex_colons(&raw[start..])
}

fn push_names(rows: &mut Vec<Row>, key: &'static str, names: &[GeneralName]) {
    for (idx, general) in names.iter().enumerate() {
        let text = match general {
            GeneralName::DNSName(n) => format!("DNS {n}"),
            GeneralName::RFC822Name(n) => format!("email {n}"),
            GeneralName::URI(n) => format!("URI {n}"),
            GeneralName::IPAddress(bytes) => format!("IP {}", ip(bytes)),
            GeneralName::DirectoryName(n) => format!("dir {n}"),
            GeneralName::RegisteredID(oid) => format!("OID {}", oid.to_id_string()),
            other => format!("{other:?}"),
        };
        rows.push(Row::new(if idx == 0 { key } else { "" }, text));
    }
}

fn ip(bytes: &[u8]) -> String {
    if let Ok(v4) = <[u8; 4]>::try_from(bytes) {
        Ipv4Addr::from(v4).to_string()
    } else if let Ok(v6) = <[u8; 16]>::try_from(bytes) {
        Ipv6Addr::from(v6).to_string()
    } else {
        der::hex_colons(bytes)
    }
}

/// Self-signed: issuer equals subject, and the authority key id (if any) is the
/// certificate's own key id.
fn is_self_signed(cert: &X509Certificate) -> bool {
    let tbs = &cert.tbs_certificate;
    if tbs.subject.as_raw() != tbs.issuer.as_raw() {
        return false;
    }
    let mut ski = None;
    let mut aki = None;
    for ext in cert.extensions() {
        match ext.parsed_extension() {
            ParsedExtension::SubjectKeyIdentifier(id) => ski = Some(id.0),
            ParsedExtension::AuthorityKeyIdentifier(a) => {
                aki = a.key_identifier.as_ref().map(|id| id.0)
            }
            _ => {}
        }
    }
    match (aki, ski) {
        (Some(a), Some(s)) => a == s,
        _ => true,
    }
}

/// How a certificate's validity looks at `now` (all Unix seconds): the colour and a
/// relative description. Expired or not yet valid is red, under 30 days left yellow,
/// otherwise green.
pub fn expiry(not_before: i64, not_after: i64, now: i64) -> (Tone, String) {
    const SOON: i64 = 30 * 86_400;
    if now < not_before {
        return (
            Tone::Red,
            format!("not valid yet, starts in {}", span(not_before - now)),
        );
    }
    if now >= not_after {
        return (Tone::Red, format!("expired {} ago", span(now - not_after)));
    }
    let left = not_after - now;
    let tone = if left < SOON {
        Tone::Yellow
    } else {
        Tone::Green
    };
    (tone, format!("expires in {}", span(left)))
}

fn span(secs: i64) -> String {
    match secs / 86_400 {
        0 => "less than a day".into(),
        1 => "1 day".into(),
        days => format!("{days} days"),
    }
}

/// Unix seconds as `YYYY-MM-DD HH:MM:SS UTC`.
pub fn format_time(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    #[test]
    fn expiry_colours() {
        let (start, end) = (1_000 * DAY, 2_000 * DAY);
        assert_eq!(
            expiry(start, end, end + 3 * DAY + 5),
            (Tone::Red, "expired 3 days ago".into())
        );
        assert_eq!(
            expiry(start, end, end - 42 * DAY),
            (Tone::Green, "expires in 42 days".into())
        );
        assert_eq!(
            expiry(start, end, end - 29 * DAY),
            (Tone::Yellow, "expires in 29 days".into())
        );
        assert_eq!(
            expiry(start, end, end - 30 * DAY),
            (Tone::Green, "expires in 30 days".into())
        );
        assert_eq!(
            expiry(start, end, end - 60),
            (Tone::Yellow, "expires in less than a day".into())
        );
        assert_eq!(
            expiry(start, end, start - DAY),
            (Tone::Red, "not valid yet, starts in 1 day".into())
        );
    }

    #[test]
    fn formats_times() {
        assert_eq!(format_time(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_time(1_748_736_000), "2025-06-01 00:00:00 UTC");
        assert_eq!(format_time(951_825_599), "2000-02-29 11:59:59 UTC");
    }

    #[test]
    fn serial_hex() {
        assert_eq!(serial(&[0x00, 0x9a, 0x01]), "9A:01");
        assert_eq!(serial(&[0x00]), "00");
    }
}
