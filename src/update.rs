//! `scat --update` / `--check-update`: replace the running binary with a GitHub release.
//!
//! Downloads go through `curl` and extraction through `tar`, both run without a shell.
//! The archive is verified against its published SHA-256 and staged in a private
//! (0700) directory next to the binary. The extracted binary must be a regular file
//! that reports the expected version before it replaces the running one.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

use semver::Version;
use sha2::{Digest, Sha256};

use crate::config::{Channel, Config};

const DEFAULT_REPO: &str = "vleeuwenmenno/smart-cat";

/// A published release. Pre-releases are the tags with a pre-release part
/// (`v1.0.0-rc.1`), which is how the release workflow marks them on GitHub.
#[derive(Debug)]
struct Release {
    tag: String,
}

pub fn run(check_only: bool, pre: bool, config: &Config) -> i32 {
    let include_pre = pre || config.update.channel == Channel::Prerelease;
    match update(check_only, include_pre) {
        Ok(()) => 0,
        Err(message) => {
            crate::report!("update: {message}");
            1
        }
    }
}

fn repo() -> String {
    std::env::var("SCAT_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_string())
}

/// The release asset built for this machine.
pub fn target() -> Option<&'static str> {
    match (std::env::consts::ARCH, std::env::consts::OS) {
        ("x86_64", "linux") => Some("x86_64-unknown-linux-musl"),
        ("aarch64", "linux") => Some("aarch64-unknown-linux-musl"),
        _ => None,
    }
}

fn update(check_only: bool, include_pre: bool) -> Result<(), String> {
    let current = Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())?;
    let (release, latest) = newest_release(include_pre)?;
    let channel = if include_pre {
        "including pre-releases"
    } else {
        "stable"
    };

    if latest <= current {
        println!("scat {current} is up to date (newest {channel}: {latest})");
        return Ok(());
    }
    if check_only {
        println!(
            "scat {latest} is available (installed: {current}); run `scat --update{}`",
            if include_pre { " --pre" } else { "" }
        );
        return Ok(());
    }

    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| format!("cannot locate the running binary: {e}"))?;
    if is_cargo_build(&exe) && std::env::var_os("SCAT_UPDATE_FORCE").is_none_or(|v| v != "1") {
        return Err(format!(
            "{} is a cargo build; rebuild it from source instead (SCAT_UPDATE_FORCE=1 replaces it anyway)",
            shown(&exe)
        ));
    }

    let target = target().ok_or("no release builds exist for this platform")?;
    let archive_name = format!("scat-{target}.tar.gz");
    let base = format!(
        "https://github.com/{}/releases/download/{}",
        repo(),
        release.tag
    );
    let archive_url = format!("{base}/{archive_name}");
    let checksum_url = format!("{archive_url}.sha256");

    let dir = exe
        .parent()
        .ok_or("the running binary has no parent directory")?;
    let work = WorkDir::create(dir).map_err(|e| {
        format!(
            "cannot write to {}: {e}\nscat was probably installed by a package manager; update it there, or reinstall with install.sh",
            shown(dir)
        )
    })?;

    println!("downloading scat {latest} ({target})");
    // The archive is hashed in memory and exactly those bytes are written into the
    // private work directory for tar, so nothing can change between check and use.
    let archive = fetch(&archive_url)
        .map_err(|e| format!("{e}\nrelease {} has no build for {target}", release.tag))?;
    let checksum = String::from_utf8_lossy(&fetch(&checksum_url)?).into_owned();
    let expected = checksum
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let actual = sha256_hex(&archive);
    if expected.len() != 64 || expected != actual {
        return Err(format!(
            "checksum mismatch for {archive_name}: expected {expected}, got {actual}"
        ));
    }
    let archive_path = work.path.join(&archive_name);
    write_new(&archive_path, &archive, 0o600)
        .map_err(|e| format!("cannot write {}: {e}", shown(&archive_path)))?;

    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive_path)
        .arg("--no-same-owner")
        .arg("-C")
        .arg(&work.path)
        .arg("scat")
        .status()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if !status.success() {
        return Err("tar failed to extract the archive".into());
    }
    let new_binary = work.path.join("scat");
    verify_binary(&new_binary, &latest)?;
    work.verify()
        .map_err(|e| format!("work directory changed: {e}"))?;

    fs::rename(&new_binary, &exe).map_err(|e| format!("cannot replace {}: {e}", shown(&exe)))?;
    println!("updated {} from {current} to {latest}", shown(&exe));
    // Completions and the man page come from the new binary; only files that are
    // already installed get rewritten.
    let _ = Command::new(&exe).arg("--refresh-completions").status();
    Ok(())
}

/// A path for a message, with control characters made visible.
fn shown(path: &Path) -> String {
    crate::render::sanitize_str(&path.to_string_lossy())
}

/// True for a binary inside a cargo `target/` directory (`target/debug/scat`,
/// `target/<triple>/release/scat`), which `make` rebuilds rather than `--update`.
fn is_cargo_build(exe: &Path) -> bool {
    let parts: Vec<&std::ffi::OsStr> = exe.parent().into_iter().flat_map(Path::iter).collect();
    let n = parts.len();
    let profile = |i: usize| {
        parts
            .get(i)
            .is_some_and(|p| *p == "debug" || *p == "release")
    };
    let target = |i: usize| parts.get(i).is_some_and(|p| *p == "target");
    n >= 2 && profile(n - 1) && (target(n - 2) || (n >= 3 && target(n - 3)))
}

/// Checks the extracted binary before it replaces the running one: a regular file
/// (not a symlink, directory or hard link) that reports exactly the expected version.
fn verify_binary(path: &Path, expected: &Version) -> Result<(), String> {
    let meta =
        fs::symlink_metadata(path).map_err(|e| format!("the archive has no scat binary: {e}"))?;
    if !meta.file_type().is_file() || meta.nlink() != 1 {
        return Err("the archive's scat is not a regular file".into());
    }
    if meta.uid() != effective_uid() {
        return Err("the extracted scat is owned by another user".into());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|e| format!("downloaded binary does not run: {e}"))?;
    let reported = String::from_utf8_lossy(&output.stdout);
    let want = format!("scat {expected}");
    if !output.status.success() || reported.trim() != want {
        return Err(format!(
            "downloaded binary reports {:?}, expected {want:?}",
            crate::render::sanitize_str(reported.trim())
        ));
    }
    Ok(())
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Creates `path` (which must not exist) with `mode` and writes `data` to it.
fn write_new(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    file.write_all(data)
}

/// Lists the releases from the repository's Atom feed. Unlike the REST API
/// (60 requests per hour without a token), the feed has no rate limit. It holds the
/// newest releases, which is all an update needs; drafts are never in it.
fn newest_release(include_pre: bool) -> Result<(Release, Version), String> {
    let url = format!("https://github.com/{}/releases.atom", repo());
    let body = fetch(&url).map_err(|e| {
        format!(
            "{e}\ncannot list releases of github.com/{}: the repository is private or does not exist",
            repo()
        )
    })?;
    let tags = feed_tags(&String::from_utf8_lossy(&body));
    if let Some(found) = newest(tags.clone(), include_pre) {
        return Ok(found);
    }
    match newest(tags, true) {
        Some((release, _)) => Err(format!(
            "no stable release yet; the newest is pre-release {}. Use --pre",
            release.tag
        )),
        None => Err("no releases found".to_string()),
    }
}

/// The release tags in an Atom feed, from the links to `/releases/tag/TAG`.
fn feed_tags(feed: &str) -> Vec<String> {
    const MARK: &str = "/releases/tag/";
    feed.match_indices(MARK)
        .filter_map(|(i, _)| {
            let rest = &feed[i + MARK.len()..];
            let tag = &rest[..rest.find(['"', '<'])?];
            Some(tag.to_string())
        })
        .collect()
}

/// The newest release. Only tags that parse as versions count, so a tag can never
/// carry anything into a download URL beyond semver's `[0-9A-Za-z.+-]`.
fn newest(tags: Vec<String>, include_pre: bool) -> Option<(Release, Version)> {
    tags.into_iter()
        .filter_map(|tag| {
            let version = Version::parse(tag.strip_prefix('v').unwrap_or(&tag)).ok()?;
            Some((Release { tag }, version))
        })
        .filter(|(_, v)| include_pre || v.pre.is_empty())
        .max_by(|a, b| a.1.cmp(&b.1))
}

fn curl() -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--proto", "=https", "--tlsv1.2", "--retry", "2"])
        .args([
            "-H",
            concat!("User-Agent: scat/", env!("CARGO_PKG_VERSION")),
        ]);
    cmd
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let output = curl()
        .arg(url)
        .output()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "download failed: {url}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A temporary directory next to the binary, so the final rename stays on one
/// filesystem and is atomic. Removed when dropped.
struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    /// Creates a fresh directory with an unpredictable name and mode 0700, so no other
    /// user can read, replace or add files in it, even when `parent` is group-writable.
    fn create(parent: &Path) -> std::io::Result<Self> {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        for _ in 0..16 {
            let path = parent.join(format!(
                ".scat-update-{}-{:016x}",
                std::process::id(),
                random()
            ));
            match builder.create(&path) {
                Ok(()) => {
                    let work = Self { path };
                    // The umask may have removed bits; the mode must be exactly 0700.
                    fs::set_permissions(&work.path, fs::Permissions::from_mode(0o700))?;
                    work.verify()?;
                    return Ok(work);
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
        Err(std::io::Error::other(
            "cannot create a unique work directory",
        ))
    }

    /// Checks the directory is still ours: a real directory, owned by this user, and
    /// closed to everyone else.
    fn verify(&self) -> std::io::Result<()> {
        let meta = fs::symlink_metadata(&self.path)?;
        if !meta.file_type().is_dir()
            || meta.uid() != effective_uid()
            || meta.permissions().mode() & 0o077 != 0
        {
            return Err(std::io::Error::other(format!(
                "{} is not a private directory",
                shown(&self.path)
            )));
        }
        Ok(())
    }
}

/// 64 unpredictable bits: std's per-process random hash keys, mixed with the time.
fn random() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    hasher.write_u128(now.as_nanos());
    hasher.finish()
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_dir_is_private_and_unpredictable() {
        let work = WorkDir::create(&std::env::temp_dir()).unwrap();
        let meta = fs::symlink_metadata(&work.path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        let other = WorkDir::create(&std::env::temp_dir()).unwrap();
        assert_ne!(work.path, other.path);
    }

    #[test]
    fn staged_binary_must_be_a_regular_file_with_the_right_version() {
        let work = WorkDir::create(&std::env::temp_dir()).unwrap();
        let version = Version::parse("9.8.7").unwrap();

        let link = work.path.join("link");
        std::os::unix::fs::symlink("/bin/true", &link).unwrap();
        assert!(verify_binary(&link, &version).is_err());
        let dir = work.path.join("dir");
        fs::create_dir(&dir).unwrap();
        assert!(verify_binary(&dir, &version).is_err());

        let script = work.path.join("scat");
        write_new(&script, b"#!/bin/sh\necho 'scat 9.8.7'\n", 0o700).unwrap();
        // A freshly written executable can briefly be busy if another test thread forks.
        let mut result = verify_binary(&script, &version);
        for _ in 0..20 {
            if result.as_ref().is_err_and(|e| e.contains("busy")) {
                std::thread::sleep(std::time::Duration::from_millis(50));
                result = verify_binary(&script, &version);
            }
        }
        assert_eq!(result, Ok(()));
        let wrong = Version::parse("9.8.8").unwrap();
        assert!(verify_binary(&script, &wrong).is_err());
    }

    #[test]
    fn cargo_builds_are_recognised() {
        assert!(is_cargo_build(Path::new("/src/scat/target/debug/scat")));
        assert!(is_cargo_build(Path::new("/src/scat/target/release/scat")));
        assert!(is_cargo_build(Path::new(
            "/src/target/x86_64-unknown-linux-musl/release/scat"
        )));
        assert!(!is_cargo_build(Path::new("/home/u/.local/bin/scat")));
        assert!(!is_cargo_build(Path::new("/opt/release/scat")));
    }

    #[test]
    fn releases_come_from_the_feed() {
        let feed = r#"<feed>
  <entry><link rel="alternate" type="text/html" href="https://github.com/o/r/releases/tag/v0.2.0-rc.1"/></entry>
  <entry><link rel="alternate" type="text/html" href="https://github.com/o/r/releases/tag/v0.1.1"/></entry>
  <entry><link rel="alternate" type="text/html" href="https://github.com/o/r/releases/tag/nightly"/></entry>
  <entry><link rel="alternate" type="text/html" href="https://github.com/o/r/releases/tag/v0.1.0"/></entry>
</feed>"#;
        let tags = feed_tags(feed);
        assert_eq!(tags, ["v0.2.0-rc.1", "v0.1.1", "nightly", "v0.1.0"]);
        let (release, stable) = newest(tags.clone(), false).unwrap();
        assert_eq!(
            (release.tag.as_str(), stable.to_string().as_str()),
            ("v0.1.1", "0.1.1")
        );
        let (release, _) = newest(tags, true).unwrap();
        assert_eq!(release.tag, "v0.2.0-rc.1");
        assert!(newest(vec!["../../evil".into()], true).is_none());
    }
}
