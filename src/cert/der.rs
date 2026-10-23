//! Small decoders for the certificate view: a DER TLV reader, base64 and the OpenSSH
//! wire format. Only what is needed to describe keys; nothing here validates them.

/// One DER element: its tag byte and its contents.
#[derive(Debug, Clone, Copy)]
pub struct Tlv<'a> {
    pub tag: u8,
    pub value: &'a [u8],
}

pub const SEQUENCE: u8 = 0x30;
pub const INTEGER: u8 = 0x02;
pub const BIT_STRING: u8 = 0x03;
pub const OCTET_STRING: u8 = 0x04;
pub const OID: u8 = 0x06;

/// Reads one element with a single-byte tag; returns it and the bytes after it.
pub fn read(input: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    let (&tag, rest) = input.split_first()?;
    if tag & 0x1f == 0x1f {
        return None;
    }
    let (&first, mut rest) = rest.split_first()?;
    let len = if first < 0x80 {
        usize::from(first)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let len = rest[..count]
            .iter()
            .fold(0usize, |acc, &b| (acc << 8) | usize::from(b));
        rest = &rest[count..];
        len
    };
    if rest.len() < len {
        return None;
    }
    Some((
        Tlv {
            tag,
            value: &rest[..len],
        },
        &rest[len..],
    ))
}

/// Reads one element and requires its tag.
pub fn expect(input: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    let (tlv, rest) = read(input)?;
    (tlv.tag == tag).then_some((tlv.value, rest))
}

/// The elements inside a constructed value.
pub fn items(mut value: &[u8]) -> Option<Vec<Tlv<'_>>> {
    let mut out = Vec::new();
    while !value.is_empty() {
        let (tlv, rest) = read(value)?;
        out.push(tlv);
        value = rest;
    }
    Some(out)
}

/// The contents of a BIT STRING without its unused-bits byte.
pub fn bit_string(value: &[u8]) -> Option<&[u8]> {
    match value.split_first() {
        Some((0, bits)) => Some(bits),
        _ => None,
    }
}

/// Size in bits of an unsigned big-endian integer (leading zero bytes ignored).
pub fn int_bits(value: &[u8]) -> u32 {
    let trimmed = match value.iter().position(|&b| b != 0) {
        Some(start) => &value[start..],
        None => return 0,
    };
    (trimmed.len() as u32 - 1) * 8 + (8 - trimmed[0].leading_zeros())
}

/// An OID in dotted form, for algorithms scat has no name for.
pub fn oid_string(value: &[u8]) -> String {
    let mut parts = Vec::new();
    let mut acc: u64 = 0;
    for (idx, &byte) in value.iter().enumerate() {
        acc = (acc << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            if parts.is_empty() {
                let first = (acc / 40).min(2);
                parts.push(first);
                parts.push(acc - first * 40);
            } else {
                parts.push(acc);
            }
            acc = 0;
        } else if idx + 1 == value.len() {
            return "?".into();
        }
    }
    parts
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Decodes standard base64, skipping whitespace. Padding is optional.
pub fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut padding = false;
    for &c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding = true;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        if padding {
            return None;
        }
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Standard base64 without padding, as in OpenSSH fingerprints.
pub fn base64_encode_unpadded(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// Bytes as colon-separated upper-case hex, like `openssl x509 -fingerprint`.
pub fn hex_colons(data: &[u8]) -> String {
    data.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Reader for the OpenSSH wire format (RFC 4251): u32 lengths and strings.
pub struct Wire<'a>(pub &'a [u8]);

impl<'a> Wire<'a> {
    pub fn u32(&mut self) -> Option<u32> {
        let (head, rest) = self.0.split_first_chunk::<4>()?;
        self.0 = rest;
        Some(u32::from_be_bytes(*head))
    }

    pub fn byte(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }

    pub fn string(&mut self) -> Option<&'a [u8]> {
        let len = self.u32()? as usize;
        if self.0.len() < len {
            return None;
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Some(head)
    }

    pub fn str(&mut self) -> Option<&'a str> {
        std::str::from_utf8(self.string()?).ok()
    }
}

/// Appends an OpenSSH string (u32 length and bytes).
pub fn put_string(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        assert_eq!(base64_decode(b"aGVs\nbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode(b"aGVsbG8").unwrap(), b"hello");
        assert!(base64_decode(b"a$b").is_none());
        assert_eq!(base64_encode_unpadded(b"hello"), "aGVsbG8");
        assert_eq!(base64_encode_unpadded(b"hell"), "aGVsbA");
        assert_eq!(base64_encode_unpadded(b"hel"), "aGVs");
    }

    #[test]
    fn reads_der() {
        let (tlv, rest) = read(&[0x02, 0x02, 0x01, 0x00, 0xff]).unwrap();
        assert_eq!(
            (tlv.tag, tlv.value, rest),
            (INTEGER, &[1u8, 0][..], &[0xffu8][..])
        );
        assert!(read(&[0x02, 0x05, 0x01]).is_none());
        let long = [&[0x04, 0x81, 0x80][..], &[7u8; 128]].concat();
        assert_eq!(read(&long).unwrap().0.value.len(), 128);
    }

    #[test]
    fn integer_bits_and_oids() {
        assert_eq!(int_bits(&[0x00, 0x80, 0x00]), 16);
        assert_eq!(int_bits(&[0x01]), 1);
        assert_eq!(
            oid_string(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]),
            "1.2.840.10045.2.1"
        );
        assert_eq!(hex_colons(&[0xab, 0x01]), "AB:01");
    }
}
