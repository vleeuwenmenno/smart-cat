//! Thumbnails for the tiles view: where they come from, freedesktop's shared thumbnail
//! cache, and scat's own cache.
//!
//! Lookup order for one file: a fresh freedesktop thumbnail (made by a file manager),
//! then scat's cache, then generating one. Generated thumbnails (and "this file has
//! none") are stored in scat's cache, keyed by path, size and modification time.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::RgbaImage;
use sha2::{Digest, Sha256};

use super::md5;

/// Thumbnails are made and cached at this size (the longer side, in pixels), the size
/// of freedesktop's "large" thumbnails.
pub const THUMB_SIDE: u32 = 256;

/// Images and PDFs larger than this get an icon instead of a thumbnail.
pub const MAX_SOURCE_BYTES: u64 = 64 << 20;

/// Time limit for one video poster frame (ffmpeg) or PDF page (sandboxed helper).
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
    if let Some(file) = &file {
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
    let Ok(data) = read_capped(file, MAX_THUMB_FILE) else {
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
        Source::Audio => match crate::audio::cover_art_bytes(target.path) {
            Some(data) => decode(data, false),
            None => Made::Nothing,
        },
    }
}

fn read_capped(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let mut data = Vec::new();
    File::open(path)?.take(max + 1).read_to_end(&mut data)?;
    if data.len() as u64 > max {
        return Err(std::io::Error::other("too large"));
    }
    Ok(data)
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
        let Ok(data) = read_capped(&dir.join(size).join(&name), MAX_THUMB_FILE) else {
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
}
