# Certificates and keys

In a terminal, `scat` shows a decoded summary of X.509 certificates, certificate requests, public keys and SSH key files. Private keys are summarised **without printing any key material**.

```
$ scat fullchain.pem
Certificate 1 of 3
  subject         CN=leaf.scat.test, O=scat tests
  issuer          CN=scat Test Intermediate, O=scat tests
  serial          2A:2B:2C
  not before      2025-06-01 00:00:00 UTC
  not after       2035-06-01 00:00:00 UTC (expires in 3142 days)
  SANs            DNS leaf.scat.test
                  DNS *.scat.test
                  IP 192.0.2.7
                  email admin@scat.test
  key             RSA 2048 bits
  signature       ecdsa-with-SHA256
  CA              no
  key usage       digital signature, key encipherment (critical)
  extended usage  server auth, client auth
  SHA-256         9E:70:7E:3A:…:4F:76
  SHA-1           5F:2D:D0:6E:…:C5:8D
  self-signed     no

Certificate 2 of 3
…

chain: #1 leaf.scat.test → #2 scat Test Intermediate → #3 scat Test Root CA (self-signed)
```

Piped output is the file itself, byte for byte, like `cat`: `scat key.pem > copy` copies the key.

## What is recognised

| File | Shown |
|---|---|
| PEM `CERTIFICATE` (also `TRUSTED CERTIFICATE`), bundles with several | subject, issuer, serial, validity, SANs (DNS, IP, email, URI), key algorithm and size or curve, signature algorithm, CA flag and path length, key usage, extended key usage, SHA-256 and SHA-1 fingerprints, self-signed |
| DER certificate (`.der`, `.cer`, `.crt`) | the same |
| `CERTIFICATE REQUEST` (CSR) | subject, requested SANs, key, signature algorithm |
| `PUBLIC KEY`, `RSA PUBLIC KEY` | key type, size or curve, OpenSSH `SHA256:` fingerprint |
| OpenSSH `.pub`, `authorized_keys`, `known_hosts` | one line per key: type, bits, `SHA256:` fingerprint (same as `ssh-keygen -lf`), comment; host patterns for `known_hosts` (hashed hosts as `(hashed)`), options for `authorized_keys` (dim) |
| `PRIVATE KEY`, `RSA PRIVATE KEY`, `EC PRIVATE KEY`, `DSA PRIVATE KEY`, `ENCRYPTED PRIVATE KEY`, `OPENSSH PRIVATE KEY` | see [Private keys](#private-keys) |

Detection looks at the first 8 KiB: a `-----BEGIN …-----` line for one of the labels above, or a first line that is an OpenSSH public key. This works whatever the extension is among `.pem .crt .cer .cert .der .key .pub .csr .req .p8 .pk8 .txt` and for files without an extension (`id_ed25519`, `cert`). Files named `*.pub`, `authorized_keys*` and `known_hosts*` are always tried. Other files, such as source code or YAML with an embedded certificate, keep their usual view; a [rule](configuration.md) with `renderer = "cert"` turns the summary on for them.

### Validity colours

With colours on, the `not after` line is:

- red when the certificate has expired (`expired 3 days ago`) or is not valid yet;
- yellow when less than 30 days are left (`expires in 12 days`);
- green otherwise.

### Bundles and chains

Each certificate gets a numbered title (`Certificate 2 of 3`). When the issuer of one certificate is the subject of the next, a dim `chain:` line at the end shows the links with `→`; certificates that do not link are separated by `;`.

### Text around PEM blocks

Files that mix text and PEM blocks (notes, `openssl x509 -text` output, `Bag Attributes` from `openssl pkcs12`) are shown in order: text lines are printed as they are (with control characters made safe), and each PEM block is replaced by its summary. Blocks scat does not summarise (`DH PARAMETERS`, `X509 CRL`, PGP public keys, …) are printed as text.

## Private keys

The rendered view never prints private key material, not even part of the base64 body. It shows only what can be read without the key itself:

```
$ scat ~/.ssh/id_ed25519
Private key
  key          ED25519 256 bits
  format       OpenSSH
  encrypted    no
  fingerprint  SHA256:ye6cAp7aB4kY7igETRN0NKKnC3i1+nVWavNBs86ro9M (public key)
  comment      test@scat
  private key material hidden; use --raw or a pipe to print it
```

- **key**: type and size or curve, when the file says so without decryption (OpenSSH keys always do; encrypted PKCS#8 keys do not).
- **encrypted**: `no`, or the scheme (`aes256-ctr (bcrypt)`, `PBES2 (PBKDF2, AES-256-CBC)`, `AES-256-CBC` for legacy `Proc-Type: 4,ENCRYPTED` PEM).
- **fingerprint**: the OpenSSH `SHA256:` fingerprint of the public half, when it is stored in the file (OpenSSH keys, even encrypted; RSA; EC keys with their public point). It matches `ssh-keygen -lf`.
- **comment**: OpenSSH keys that are not encrypted.

Any PEM block whose label contains `PRIVATE KEY` is hidden, including blocks that do not parse, blocks without an `END` line, and `PGP PRIVATE KEY BLOCK`. A file with a private key in the middle of other text shows the text and hides only the key.

Hiding also covers every case where `auto` (or `cert`) shows a key file as text or as a hex dump instead of the summary:

- files over 4 MiB, files with bytes that are not UTF-8, and files whose blocks do not parse;
- files with a known extension (or none) where the `BEGIN` line comes after the first 8 KiB, so the summary is not chosen;
- broken PEM: a private key inside an unfinished certificate, a lowercase or oddly spaced label (`-----begin private key-----`), a marker after other text on the line, CRLF line ends.

In these views the `BEGIN` line stays and the body is replaced by one line, `(private key material hidden; use --raw or a pipe to print it)`, up to the matching `END` line. Without a matching `END` line everything after the `BEGIN` line is hidden. Line numbers in such a text view count the shown lines, not the lines of the file.

### Seeing the raw file

| Command | Prints |
|---|---|
| `scat --raw key.pem` | the file exactly, like `cat` |
| `scat key.pem \| less`, `scat key.pem > copy` | the file exactly (output is not a terminal) |
| rule `renderer = "code"` | the PEM text, syntax highlighted, **including the key** |
| rule `renderer = "text"` | the PEM text, plain, **including the key** |
| rule `renderer = "cert"` or `"auto"` | the summary, key hidden |

Only the `auto` and `cert` renderers hide key material. Choosing `code` or `text` in a rule is asking for the text, so it prints like `cat`. The same is true for files outside the detected extensions, which never get the summary unless a rule asks for it.

## Configuration

```toml
# Never summarise certificates; show the PEM text instead.
[[rule]]
match = { ext = ["pem", "crt"] }
renderer = "code"

# Summarise PEM files with an extension scat does not try by itself.
[[rule]]
match = { ext = "chain" }
renderer = "cert"
```

`renderer = "cert"` on a file with nothing recognisable shows it as text (or a hex dump for binary files), without an error. The same happens when a certificate does not parse. Private key bodies stay hidden in that text or hex dump.

## Limits

- Files over 4 MiB are shown as text, with private key bodies hidden.
- Key markers are found per line; in a line longer than 64 KiB a marker that spans a 64 KiB boundary is not seen.
- Signatures are not verified; `self-signed` means the issuer equals the subject (and the authority key id, if any, is the certificate's own).
- PGP/GPG key blocks are not decoded (public ones are shown as text, private ones are hidden).
- PKCS#12 (`.p12`, `.pfx`) and PKCS#7 bundles are not decoded.
- Ed25519 keys in PKCS#8 do not store the public key, so no fingerprint is shown for them.
