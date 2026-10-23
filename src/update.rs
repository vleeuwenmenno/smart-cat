//! `scat --update` / `--check-update`: replace the running binary with a GitHub release.
//!
//! Downloads go through `curl` and extraction through `tar`, both run without a shell.
//! The archive is verified against its published SHA-256 before anything is replaced.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::{Channel, Config};

const DEFAULT_REPO: &str = "vleeuwenmenno/smart-cat";

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    prerelease: bool,
    draft: bool,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub fn run(check_only: bool, pre: bool, config: &Config) -> i32 {
    let include_pre = pre || config.update.channel == Channel::Prerelease;
    match update(check_only, include_pre) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("scat: update: {message}");
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

    let target = target().ok_or("no release builds exist for this platform")?;
    let archive_name = format!("scat-{target}.tar.gz");
    let asset_url = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.browser_download_url.clone())
            .ok_or_else(|| format!("release {} has no asset {name}", release.tag_name))
    };
    let archive_url = asset_url(&archive_name)?;
    let checksum_url = asset_url(&format!("{archive_name}.sha256"))?;

    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| format!("cannot locate the running binary: {e}"))?;
    let dir = exe
        .parent()
        .ok_or("the running binary has no parent directory")?;
    let work = WorkDir::create(dir).map_err(|e| {
        format!(
            "cannot write to {}: {e}\nscat was probably installed by a package manager; update it there, or reinstall with install.sh",
            dir.display()
        )
    })?;

    println!("downloading scat {latest} ({target})");
    let archive = work.path.join(&archive_name);
    download(&archive_url, &archive)?;
    let checksum = String::from_utf8_lossy(&fetch(&checksum_url)?).into_owned();
    let expected = checksum
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let actual = sha256_hex(&archive)?;
    if expected.len() != 64 || expected != actual {
        return Err(format!(
            "checksum mismatch for {archive_name}: expected {expected}, got {actual}"
        ));
    }

    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&work.path)
        .arg("scat")
        .status()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if !status.success() {
        return Err("tar failed to extract the archive".into());
    }
    let new_binary = work.path.join("scat");
    let output = Command::new(&new_binary)
        .arg("--version")
        .output()
        .map_err(|e| format!("downloaded binary does not run: {e}"))?;
    if !output.status.success() {
        return Err("downloaded binary does not run".into());
    }

    fs::rename(&new_binary, &exe).map_err(|e| format!("cannot replace {}: {e}", exe.display()))?;
    println!("updated {} from {current} to {latest}", exe.display());
    Ok(())
}

fn newest_release(include_pre: bool) -> Result<(Release, Version), String> {
    let url = format!(
        "https://api.github.com/repos/{}/releases?per_page=30",
        repo()
    );
    let body = fetch(&url).map_err(|e| {
        format!(
            "{e}\ncannot list releases of github.com/{}: the repository is private or does not exist",
            repo()
        )
    })?;
    let releases: Vec<Release> = serde_json::from_slice(&body)
        .map_err(|e| format!("unexpected response from GitHub: {e}"))?;
    releases
        .into_iter()
        .filter(|r| !r.draft && (include_pre || !r.prerelease))
        .filter_map(|r| {
            let version = Version::parse(r.tag_name.trim_start_matches('v')).ok()?;
            Some((r, version))
        })
        .max_by(|a, b| a.1.cmp(&b.1))
        .ok_or_else(|| "no releases found".to_string())
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
        .args(["-H", "Accept: application/vnd.github+json", url])
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

fn download(url: &str, dest: &Path) -> Result<(), String> {
    let status = curl()
        .arg("-o")
        .arg(dest)
        .arg(url)
        .status()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("download failed: {url}"))
    }
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// A temporary directory next to the binary, so the final rename stays on one
/// filesystem and is atomic. Removed when dropped.
struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    fn create(parent: &Path) -> std::io::Result<Self> {
        let path = parent.join(format!(".scat-update-{}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
