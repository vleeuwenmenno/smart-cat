//! Thumbnails for the tiles view: where they come from, freedesktop's shared thumbnail
//! cache, and scat's own cache.
//!
//! Lookup order for one file: a fresh freedesktop thumbnail (made by a file manager),
//! then scat's cache, then generating one. Generated thumbnails (and "this file has
//! none") are stored in scat's cache, keyed by path, size and modification time.
//!
//! Cache files are read and written defensively. scat's cache directory is used only
//! when it is a real directory (not a symlink) owned by the user with mode 0700, and
//! its parent is not writable by others. Every cache file is opened with `O_NOFOLLOW |
//! O_NONBLOCK` (a FIFO cannot block the open) and must be a regular file of a sane size;
//! files in scat's own cache must also be owned by the user. [`prune`] keeps the cache
//! small with the same rules.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::RgbaImage;
use sha2::{Digest, Sha256};

use super::md5;

/// Thumbnails are made and cached at this size (the longer side, in pixels), the size
/// of freedesktop's "large" thumbnails.
pub const THUMB_SIDE: u32 = 256;

/// Images, PDFs and fonts larger than this get an icon instead of a thumbnail.
pub const MAX_SOURCE_BYTES: u64 = 64 << 20;

/// Time limit for one video poster frame (ffmpeg), PDF page or font (sandboxed helpers).
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(3);

/// Largest cached or freedesktop thumbnail file read back.
const MAX_THUMB_FILE: u64 = 16 << 20;

/// Bump when the stored thumbnails change, so old entries are not used.
const CACHE_VERSION: &str = "1";

/// What a thumbnail is made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Image { svg: bool },
    Video,
    Pdf,
    Audio,
    Font,
}

/// The thumbnail source for a file name, by extension. Files without a known
/// extension get an icon; sniffing every file of a big directory costs too much.
pub fn source_for(name: &str) -> Option<Source> {
    let (_, ext) = name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "jpe" | "jfif" | "gif" | "webp" | "bmp" | "ico" | "tif"
        | "tiff" | "qoi" | "tga" | "pnm" | "pbm" | "pgm" | "ppm" | "pam" => {
            Source::Image { svg: false }
        }
        "svg" => Source::Image { svg: true },
        "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "wmv" | "flv" | "mpg" | "mpeg" | "ts"
        | "m2ts" | "ogv" | "3gp" => Source::Video,
        "mp3" | "flac" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "wav" | "aiff" | "aif" | "wv"
        | "ape" | "mpc" | "spx" => Source::Audio,
        "pdf" => Source::Pdf,
        "ttf" | "otf" | "ttc" | "otc" | "woff" | "woff2" => Source::Font,
        _ => return None,
    })
}

/// The thumbnail caches to read and write.
#[derive(Debug, Clone, Default)]
pub struct Cache {
    /// scat's own thumbnails: `$XDG_CACHE_HOME/scat/thumbnails`.
    own: Option<PathBuf>,
    /// freedesktop's shared cache: `$XDG_CACHE_HOME/thumbnails`.
    shared: Option<PathBuf>,
}

impl Cache {
    /// Both caches under the user's cache directory, or none when `enabled` is false.
    pub fn new(enabled: bool) -> Cache {
        if !enabled {
            return Cache::default();
        }
        Cache::under(cache_home())
    }

    /// Both caches under `home` (the cache directory, e.g. `~/.cache`).
    pub fn under(home: Option<PathBuf>) -> Cache {
        match home {
            Some(home) => Cache {
                own: Some(home.join("scat").join("thumbnails")),
                shared: Some(home.join("thumbnails")),
            },
            None => Cache::default(),
        }
    }
}

/// `$XDG_CACHE_HOME` when it is an absolute path, else `$HOME/.cache`.
fn cache_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        && dir.is_absolute()
    {
        return Some(dir);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .map(|h| h.join(".cache"))
}

/// The file a thumbnail is made for.
#[derive(Debug, Clone)]
pub struct Target<'a> {
    /// Absolute path.
    pub path: &'a Path,
    pub size: u64,
    pub modified: SystemTime,
}

/// The thumbnail for `target`, from a cache or freshly made. `None` means the file
/// gets an icon.
pub fn thumbnail(cache: &Cache, target: &Target, source: Source) -> Option<RgbaImage> {
    if let Some(img) = freedesktop(cache, target) {
        return Some(img);
    }
    let file = cache
        .own
        .as_ref()
        .map(|dir| dir.join(format!("{}.png", key(target))));
    let readable = cache.own.as_deref().is_some_and(trusted_dir);
    if let Some(file) = file.as_ref().filter(|_| readable) {
        match read_own(file) {
            Lookup::Hit(img) => return Some(img),
            Lookup::Nothing => return None,
            Lookup::Miss => {}
        }
    }
    let made = generate(target, source);
    if let (Some(file), Some(dir)) = (&file, &cache.own) {
        match &made {
            Made::Image(img) => store(dir, file, Some(img)),
            Made::Nothing => store(dir, file, None),
            Made::Unavailable => {}
        }
    }
    match made {
        Made::Image(img) => Some(img),
        _ => None,
    }
}

/// Cache key: a hash of the path, size, modification time and thumbnail size.
pub fn key(target: &Target) -> String {
    let mtime = target
        .modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(CACHE_VERSION.as_bytes());
    hasher.update([0]);
    hasher.update(target.path.as_os_str().as_bytes());
    hasher.update([0]);
    hasher.update(
        format!(
            "{}:{}.{:09}:{THUMB_SIDE}",
            target.size,
            mtime.as_secs(),
            mtime.subsec_nanos()
        )
        .as_bytes(),
    );
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug)]
enum Lookup {
    Hit(RgbaImage),
    /// Cached "no thumbnail" (an empty file).
    Nothing,
    Miss,
}

fn read_own(file: &Path) -> Lookup {
    let Ok(data) = read_cache_file(file, MAX_THUMB_FILE, true) else {
        return Lookup::Miss;
    };
    if data.is_empty() {
        return Lookup::Nothing;
    }
    match crate::image::thumbnail(data, false, THUMB_SIDE) {
        Ok(img) => Lookup::Hit(img),
        Err(_) => Lookup::Miss,
    }
}

/// Writes a thumbnail (or an empty file for "none") atomically: directory mode 0700,
/// file mode 0600. Failures are ignored; the cache is only an optimisation.
fn store(dir: &Path, file: &Path, img: Option<&RgbaImage>) {
    let mut data = Vec::new();
    if let Some(img) = img {
        let encoder = image::codecs::png::PngEncoder::new_with_quality(
            &mut data,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        );
        if image::ImageEncoder::write_image(
            encoder,
            img,
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .is_err()
        {
            return;
        }
    }
    if fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .is_err()
        || !trusted_dir(dir)
    {
        return;
    }
    let tmp = dir.join(format!(
        ".{}.{}.{:?}.tmp",
        file.file_name()
            .map_or_else(Default::default, |n| n.to_string_lossy().into_owned()),
        std::process::id(),
        std::thread::current().id()
    ));
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)
        .and_then(|mut f| f.write_all(&data));
    if written.is_err() || fs::rename(&tmp, file).is_err() {
        let _ = fs::remove_file(&tmp);
    }
}

enum Made {
    Image(RgbaImage),
    /// The file has no thumbnail (no cover art, cannot be decoded): cached.
    Nothing,
    /// Not made this time (tool missing, file too large): not cached.
    Unavailable,
}

fn generate(target: &Target, source: Source) -> Made {
    let decode = |data: Vec<u8>, svg: bool| match crate::image::thumbnail(data, svg, THUMB_SIDE) {
        Ok(img) => Made::Image(img),
        Err(_) => Made::Nothing,
    };
    match source {
        Source::Image { svg } => {
            if target.size > MAX_SOURCE_BYTES {
                return Made::Unavailable;
            }
            match read_capped(target.path, MAX_SOURCE_BYTES) {
                Ok(data) => decode(data, svg),
                Err(_) => Made::Unavailable,
            }
        }
        Source::Pdf => {
            if target.size > MAX_SOURCE_BYTES {
                return Made::Unavailable;
            }
            match crate::pdf::render_page_within(
                target.path,
                1,
                (THUMB_SIDE, THUMB_SIDE),
                TOOL_TIMEOUT,
            ) {
                Ok(png) => decode(png, false),
                Err(_) => Made::Nothing,
            }
        }
        Source::Video => {
            if crate::util::which("ffmpeg").is_none() {
                return Made::Unavailable;
            }
            match crate::video::thumbnail_png(target.path, THUMB_SIDE, TOOL_TIMEOUT) {
                Some(png) => decode(png, false),
                None => Made::Nothing,
            }
        }
        Source::Font => {
            if target.size > MAX_SOURCE_BYTES {
                return Made::Unavailable;
            }
            match read_capped(target.path, MAX_SOURCE_BYTES) {
                Ok(data) => match crate::font::thumbnail(data, THUMB_SIDE, TOOL_TIMEOUT) {
                    Some(img) => Made::Image(img),
                    None => Made::Nothing,
                },
                Err(_) => Made::Unavailable,
            }
        }
        Source::Audio => match crate::audio::cover_art_bytes(target.path) {
            Some(data) => decode(data, false),
            None => Made::Nothing,
        },
    }
}

/// Reads a source file (symlinks followed: the thumbnail is of the target). Opened
/// non-blocking and read only when it is a regular file, so a FIFO swapped in after
/// the listing cannot hang scat.
fn read_capped(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    read_regular(file, max, false)
}

/// Reads a cache file: not through a symlink, non-blocking, a regular file of at most
/// `max` bytes, and owned by the user when `own`.
fn read_cache_file(path: &Path, max: u64, own: bool) -> std::io::Result<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    read_regular(file, max, own)
}

fn read_regular(file: File, max: u64, own: bool) -> std::io::Result<Vec<u8>> {
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    if own && meta.uid() != uid() {
        return Err(std::io::Error::other("not owned by the user"));
    }
    if meta.len() > max {
        return Err(std::io::Error::other("too large"));
    }
    let mut data = Vec::new();
    file.take(max + 1).read_to_end(&mut data)?;
    if data.len() as u64 > max {
        return Err(std::io::Error::other("too large"));
    }
    Ok(data)
}

fn uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Whether scat's cache directory `dir` may be used: a real directory owned by the user
/// with no access for others, inside a parent that is a real directory others cannot
/// write to (so `dir` cannot be swapped for a symlink).
fn trusted_dir(dir: &Path) -> bool {
    let private = fs::symlink_metadata(dir)
        .is_ok_and(|m| m.file_type().is_dir() && m.uid() == uid() && m.mode() & 0o077 == 0);
    let parent_safe = dir.parent().is_some_and(|parent| {
        fs::symlink_metadata(parent)
            .is_ok_and(|m| m.file_type().is_dir() && m.uid() == uid() && m.mode() & 0o022 == 0)
    });
    private && parent_safe
}

/// A thumbnail from freedesktop's shared cache, when one exists for this file and its
/// `Thumb::MTime` matches the file's modification time.
fn freedesktop(cache: &Cache, target: &Target) -> Option<RgbaImage> {
    let dir = cache.shared.as_ref()?;
    let uri = file_uri(target.path);
    let name = format!("{}.png", md5::hex(uri.as_bytes()));
    let mtime = target
        .modified
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_secs()
        .to_string();
    for size in ["large", "x-large", "normal", "xx-large"] {
        let Ok(data) = read_cache_file(&dir.join(size).join(&name), MAX_THUMB_FILE, false) else {
            continue;
        };
        let text = png_text(&data);
        let field = |key: &str| text.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        if field("Thumb::MTime") != Some(&mtime) {
            continue;
        }
        if field("Thumb::URI").is_some_and(|u| *u != uri) {
            continue;
        }
        if let Ok(img) = crate::image::thumbnail(data, false, THUMB_SIDE) {
            return Some(img);
        }
    }
    None
}

/// Cache entries not modified for this long are removed by [`prune`].
pub const PRUNE_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// [`prune`] removes the oldest entries until the cache is at most this large.
pub const PRUNE_MAX_BYTES: u64 = 256 << 20;
/// [`prune`] runs at most this often.
pub const PRUNE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// Timestamp file of the last [`prune`], inside the cache directory.
const PRUNE_STAMP: &str = ".pruned";

/// What one [`prune_dir`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub removed: usize,
    pub kept: usize,
    pub kept_bytes: u64,
}

/// Prunes scat's thumbnail cache when the last prune was more than [`PRUNE_INTERVAL`]
/// ago (by the timestamp file's modification time): entries older than [`PRUNE_AGE`]
/// go, then the oldest until the rest fits [`PRUNE_MAX_BYTES`]. Cheap when not due: one
/// `lstat`. Errors are ignored; the cache is only an optimisation.
pub fn prune(cache: &Cache) {
    let Some(dir) = cache.own.as_deref() else {
        return;
    };
    let now = SystemTime::now();
    let stamp = dir.join(PRUNE_STAMP);
    if let Ok(meta) = fs::symlink_metadata(&stamp)
        && meta.is_file()
        && let Ok(when) = meta.modified()
        && now
            .duration_since(when)
            .is_ok_and(|age| age < PRUNE_INTERVAL)
    {
        return;
    }
    if prune_dir(dir, now, PRUNE_AGE, PRUNE_MAX_BYTES).is_ok() {
        // Replaced, not rewritten: a symlink or FIFO at the stamp's name is not
        // followed or opened.
        let tmp = dir.join(format!("{PRUNE_STAMP}.{}.tmp", std::process::id()));
        let written = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)
            .and_then(|mut f| writeln!(f, "{}", unix_secs(now)));
        if written.is_err() || fs::rename(&tmp, &stamp).is_err() {
            let _ = fs::remove_file(&tmp);
        }
    }
}

fn unix_secs(time: SystemTime) -> i64 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Removes entries of the cache directory `dir` older than `max_age`, then the oldest
/// until the rest is at most `max_bytes`. Only regular files owned by the user, directly
/// in `dir`, are looked at or removed; the directory must pass [`trusted_dir`] and is
/// opened once without following symlinks, and every entry is looked at and removed
/// relative to that open directory, so nothing outside it can be reached.
pub fn prune_dir(
    dir: &Path,
    now: SystemTime,
    max_age: Duration,
    max_bytes: u64,
) -> std::io::Result<Pruned> {
    use rustix::fs::{AtFlags, FileType, Mode, OFlags};
    if !trusted_dir(dir) {
        return Err(std::io::Error::other("cache directory is not private"));
    }
    let fd = rustix::fs::open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let st = rustix::fs::fstat(&fd)?;
    if st.st_uid != uid() || st.st_mode & 0o077 != 0 {
        return Err(std::io::Error::other("cache directory is not private"));
    }
    let cutoff = unix_secs(now).saturating_sub(max_age.as_secs() as i64);
    // (mtime, size, name) of the entries that stay after the age pass.
    let mut kept: Vec<(i64, u64, std::ffi::CString)> = Vec::new();
    let mut removed = 0;
    for entry in rustix::fs::Dir::read_from(&fd)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_bytes() == b"."
            || name.to_bytes() == b".."
            || name.to_bytes() == PRUNE_STAMP.as_bytes()
        {
            continue;
        }
        let Ok(st) = rustix::fs::statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW) else {
            continue;
        };
        if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile || st.st_uid != uid() {
            continue;
        }
        let mtime = st.st_mtime;
        if mtime < cutoff {
            if rustix::fs::unlinkat(&fd, name, AtFlags::empty()).is_ok() {
                removed += 1;
            }
            continue;
        }
        kept.push((mtime, st.st_size as u64, name.to_owned()));
    }
    let mut total: u64 = kept.iter().map(|(_, size, _)| size).sum();
    if total > max_bytes {
        // Oldest first.
        kept.sort();
        let mut drop_count = 0;
        for (_, size, name) in &kept {
            if total <= max_bytes {
                break;
            }
            if rustix::fs::unlinkat(&fd, name.as_c_str(), AtFlags::empty()).is_ok() {
                total -= size;
                removed += 1;
            }
            drop_count += 1;
        }
        kept.drain(..drop_count);
    }
    Ok(Pruned {
        removed,
        kept: kept.len(),
        kept_bytes: total,
    })
}

/// `file://` URI of an absolute path, escaped like GLib's `g_filename_to_uri`.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"!$&'()*+,-./:=@_~".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// The `tEXt` chunks of a PNG file as (keyword, text) pairs, Latin-1 decoded.
pub fn png_text(data: &[u8]) -> Vec<(String, String)> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    let mut out = Vec::new();
    if !data.starts_with(SIGNATURE) {
        return out;
    }
    let mut pos = SIGNATURE.len();
    while pos + 8 <= data.len() {
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let kind = &data[pos + 4..pos + 8];
        let start = pos + 8;
        let Some(end) = start.checked_add(len).filter(|&e| e <= data.len()) else {
            break;
        };
        let body = &data[start..end];
        match kind {
            b"tEXt" => {
                if let Some(nul) = body.iter().position(|&b| b == 0) {
                    let latin1 = |s: &[u8]| s.iter().map(|&b| b as char).collect::<String>();
                    out.push((latin1(&body[..nul]), latin1(&body[nul + 1..])));
                }
            }
            b"IEND" => break,
            _ => {}
        }
        pos = end + 4;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scat-thumbs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let img = RgbaImage::from_pixel(width, height, image::Rgba(color));
        let mut data = Vec::new();
        image::ImageEncoder::write_image(
            image::codecs::png::PngEncoder::new(&mut data),
            &img,
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        data
    }

    /// Inserts `tEXt` chunks right after IHDR.
    fn with_text(png: &[u8], pairs: &[(&str, &str)]) -> Vec<u8> {
        let ihdr_end = 8 + 8 + 13 + 4;
        let mut out = png[..ihdr_end].to_vec();
        for (k, v) in pairs {
            let mut body = k.as_bytes().to_vec();
            body.push(0);
            body.extend_from_slice(v.as_bytes());
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            out.extend_from_slice(b"tEXt");
            out.extend_from_slice(&body);
            out.extend_from_slice(&[0, 0, 0, 0]); // CRC is not checked by png_text
        }
        out.extend_from_slice(&png[ihdr_end..]);
        out
    }

    #[test]
    fn sources_by_extension() {
        assert_eq!(source_for("a.PNG"), Some(Source::Image { svg: false }));
        assert_eq!(source_for("logo.svg"), Some(Source::Image { svg: true }));
        assert_eq!(source_for("clip.mkv"), Some(Source::Video));
        assert_eq!(source_for("song.flac"), Some(Source::Audio));
        assert_eq!(source_for("paper.pdf"), Some(Source::Pdf));
        assert_eq!(source_for("Inter.woff2"), Some(Source::Font));
        assert_eq!(source_for("main.rs"), None);
        assert_eq!(source_for("png"), None);
    }

    #[test]
    fn uri_escaping_matches_glib() {
        assert_eq!(
            file_uri(Path::new("/home/jens/photos/me.png")),
            "file:///home/jens/photos/me.png"
        );
        assert_eq!(
            file_uri(Path::new("/tmp/a b#c%d/ü&(x)~.png")),
            "file:///tmp/a%20b%23c%25d/%C3%BC&(x)~.png"
        );
    }

    #[test]
    fn key_changes_with_path_size_and_mtime() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let base = Target {
            path: Path::new("/a/b.png"),
            size: 10,
            modified: t,
        };
        let k = key(&base);
        assert_eq!(k.len(), 32);
        assert_eq!(k, key(&base.clone()));
        let other_path = Target {
            path: Path::new("/a/c.png"),
            ..base.clone()
        };
        let other_size = Target {
            size: 11,
            ..base.clone()
        };
        let other_time = Target {
            modified: t + Duration::from_nanos(1),
            ..base.clone()
        };
        for other in [other_path, other_size, other_time] {
            assert_ne!(k, key(&other));
        }
    }

    #[test]
    fn cache_miss_then_hit_with_private_modes() {
        use std::os::unix::fs::PermissionsExt;
        let home = temp_dir("cache");
        let file = home.join("pic.png");
        fs::write(&file, png(40, 20, [255, 0, 0, 255])).unwrap();
        let meta = fs::metadata(&file).unwrap();
        let target = Target {
            path: &file,
            size: meta.len(),
            modified: meta.modified().unwrap(),
        };
        let cache = Cache::under(Some(home.join("cache")));
        let img = thumbnail(&cache, &target, Source::Image { svg: false }).unwrap();
        assert_eq!(img.dimensions(), (40, 20));

        let dir = home.join("cache/scat/thumbnails");
        let stored = dir.join(format!("{}.png", key(&target)));
        assert!(stored.is_file());
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&stored), 0o600);

        // A hit comes from the cache: replace the cached image and see it returned.
        fs::write(&stored, png(8, 8, [0, 0, 255, 255])).unwrap();
        let img = thumbnail(&cache, &target, Source::Image { svg: false }).unwrap();
        assert_eq!(img.dimensions(), (8, 8));

        // An undecodable file is cached as "nothing".
        let bad = home.join("bad.png");
        fs::write(&bad, b"not a png").unwrap();
        let meta = fs::metadata(&bad).unwrap();
        let bad_target = Target {
            path: &bad,
            size: meta.len(),
            modified: meta.modified().unwrap(),
        };
        assert!(thumbnail(&cache, &bad_target, Source::Image { svg: false }).is_none());
        let marker = dir.join(format!("{}.png", key(&bad_target)));
        assert_eq!(fs::metadata(marker).unwrap().len(), 0);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn freedesktop_thumbnail_needs_matching_mtime() {
        let home = temp_dir("freedesktop");
        let file = home.join("photo.jpg");
        fs::write(&file, b"not decoded when a thumbnail exists").unwrap();
        let meta = fs::metadata(&file).unwrap();
        let target = Target {
            path: &file,
            size: meta.len(),
            modified: meta.modified().unwrap(),
        };
        let secs = target
            .modified
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let uri = file_uri(&file);
        let cache_home = home.join("cache");
        let large = cache_home.join("thumbnails/large");
        fs::create_dir_all(&large).unwrap();
        let thumb = large.join(format!("{}.png", md5::hex(uri.as_bytes())));

        let fixture = with_text(
            &png(16, 12, [0, 255, 0, 255]),
            &[("Thumb::URI", &uri), ("Thumb::MTime", &secs)],
        );
        assert_eq!(png_text(&fixture).len(), 2);
        fs::write(&thumb, &fixture).unwrap();
        let cache = Cache {
            own: None,
            shared: Some(cache_home.join("thumbnails")),
        };
        let img = freedesktop(&cache, &target).unwrap();
        assert_eq!(img.dimensions(), (16, 12));

        // A stale thumbnail (other mtime) is ignored.
        let stale = with_text(&png(16, 12, [0, 255, 0, 255]), &[("Thumb::MTime", "1")]);
        fs::write(&thumb, stale).unwrap();
        assert!(freedesktop(&cache, &target).is_none());
        let _ = fs::remove_dir_all(&home);
    }

    fn set_age(path: &Path, days: u64) {
        let when = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    fn private_cache(name: &str) -> (PathBuf, Cache, PathBuf) {
        let home = temp_dir(name);
        let cache = Cache::under(Some(home.join("cache")));
        let dir = home.join("cache/scat/thumbnails");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .unwrap();
        (home, cache, dir)
    }

    #[test]
    fn prune_removes_old_entries_and_caps_the_size() {
        let (home, _, dir) = private_cache("prune");
        for (name, days, size) in [
            ("old.png", 40, 10),
            ("a.png", 3, 100),
            ("b.png", 2, 100),
            ("c.png", 1, 100),
        ] {
            fs::write(dir.join(name), vec![0u8; size]).unwrap();
            set_age(&dir.join(name), days);
        }
        // Never touched: a symlink to a file outside, its target, and a subdirectory.
        let outside = home.join("outside.png");
        fs::write(&outside, b"keep me").unwrap();
        set_age(&outside, 400);
        std::os::unix::fs::symlink(&outside, dir.join("link.png")).unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/old.png"), b"x").unwrap();
        set_age(&dir.join("sub/old.png"), 400);

        let now = SystemTime::now();
        let done = prune_dir(&dir, now, PRUNE_AGE, 250).unwrap();
        // old.png by age, then a.png (the oldest) to get 300 bytes under 250.
        assert_eq!(
            done,
            Pruned {
                removed: 2,
                kept: 2,
                kept_bytes: 200
            }
        );
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["b.png", "c.png", "link.png", "sub"]);
        assert_eq!(fs::read(&outside).unwrap(), b"keep me");
        assert!(dir.join("sub/old.png").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn prune_runs_once_a_day() {
        let (home, cache, dir) = private_cache("prune-daily");
        fs::write(dir.join("old.png"), b"x").unwrap();
        set_age(&dir.join("old.png"), 40);
        prune(&cache);
        assert!(!dir.join("old.png").exists());
        assert!(dir.join(PRUNE_STAMP).is_file());
        // Due again only after a day: a new old entry stays for now.
        fs::write(dir.join("old2.png"), b"x").unwrap();
        set_age(&dir.join("old2.png"), 40);
        prune(&cache);
        assert!(dir.join("old2.png").exists());
        set_age(&dir.join(PRUNE_STAMP), 2);
        prune(&cache);
        assert!(!dir.join("old2.png").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn untrusted_cache_directories_are_not_used() {
        use std::os::unix::fs::PermissionsExt;
        let home = temp_dir("untrusted");
        let file = home.join("pic.png");
        fs::write(&file, png(4, 4, [255, 0, 0, 255])).unwrap();
        let meta = fs::metadata(&file).unwrap();
        let target = Target {
            path: &file,
            size: meta.len(),
            modified: meta.modified().unwrap(),
        };
        // The thumbnails directory is a symlink to a world-writable directory.
        let elsewhere = home.join("elsewhere");
        fs::create_dir(&elsewhere).unwrap();
        fs::set_permissions(&elsewhere, fs::Permissions::from_mode(0o777)).unwrap();
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(home.join("cache/scat"))
            .unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.join("cache/scat/thumbnails")).unwrap();
        let cache = Cache::under(Some(home.join("cache")));
        // A FIFO at the entry's name would block a plain open forever.
        let fifo = elsewhere.join(format!("{}.png", key(&target)));
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        assert!(thumbnail(&cache, &target, Source::Image { svg: false }).is_some());
        // Nothing was written through the symlink, and pruning refuses the directory.
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 1);
        assert!(
            prune_dir(
                &home.join("cache/scat/thumbnails"),
                SystemTime::now(),
                PRUNE_AGE,
                0
            )
            .is_err()
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn cache_reads_need_a_regular_file() {
        let home = temp_dir("regular");
        let fifo = home.join("fifo.png");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        // Returns at once instead of waiting for a writer.
        assert!(read_cache_file(&fifo, MAX_THUMB_FILE, false).is_err());
        assert!(read_capped(&fifo, MAX_THUMB_FILE).is_err());
        let real = home.join("real.png");
        fs::write(&real, b"data").unwrap();
        let link = home.join("link.png");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(read_cache_file(&link, MAX_THUMB_FILE, false).is_err());
        assert_eq!(
            read_cache_file(&real, MAX_THUMB_FILE, true).unwrap(),
            b"data"
        );
        assert!(read_cache_file(&real, 3, true).is_err());
        let _ = fs::remove_dir_all(&home);
    }
}
