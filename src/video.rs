//! Video info view: container, duration, bitrate and streams, read with `ffprobe`.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::kv;
use crate::util::{human_size, which};

/// ffprobe is killed when it runs longer than this.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Upper bound for ffprobe's JSON, so a misbehaving binary cannot fill memory.
const MAX_PROBE_OUTPUT: u64 = 16 * 1024 * 1024;

const HINT_MISSING: &str = "install ffmpeg (ffprobe) for stream details";
const HINT_FAILED: &str = "ffprobe could not read this file";
const HINT_TIMEOUT: &str = "ffprobe timed out reading this file";

enum Probe {
    Missing,
    Failed,
    TimedOut,
    Info(Value),
}

/// Prints the info view for a video file.
pub fn render<W: Write>(
    out: &mut W,
    path: &Path,
    file_name: &str,
    size: Option<u64>,
    mime: &str,
    colors: bool,
) -> io::Result<()> {
    let probe = probe(path);
    let (rows, hint) = match &probe {
        Probe::Info(json) => (rows_from_json(json, size), None),
        Probe::Missing => (basic_rows(mime, size), Some(HINT_MISSING)),
        Probe::Failed => (basic_rows(mime, size), Some(HINT_FAILED)),
        Probe::TimedOut => (basic_rows(mime, size), Some(HINT_TIMEOUT)),
    };
    kv::write_table(out, file_name, &rows, colors)?;
    if let Some(hint) = hint {
        let (dim, reset) = if colors {
            ("\x1b[2m", "\x1b[0m")
        } else {
            ("", "")
        };
        writeln!(out, "  {dim}{hint}{reset}")?;
    }
    Ok(())
}

fn basic_rows(mime: &str, size: Option<u64>) -> Vec<(&'static str, String)> {
    vec![
        ("type", mime.to_string()),
        ("size", size.map(human_size).unwrap_or_default()),
    ]
}

fn probe(path: &Path) -> Probe {
    let Some(ffprobe) = which("ffprobe") else {
        return Probe::Missing;
    };
    // The `file:` prefix stops ffprobe from reading the name as a URL or an option.
    let mut target = std::ffi::OsString::from("file:");
    match std::path::absolute(path) {
        Ok(abs) => target.push(abs),
        Err(_) => target.push(path),
    }
    let child = Command::new(ffprobe)
        .args(["-v", "error", "-print_format", "json"])
        .args(["-show_format", "-show_streams", "-i"])
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return Probe::Failed;
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Probe::Failed;
    };
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.take(MAX_PROBE_OUTPUT).read_to_end(&mut buf);
        buf
    });

    let deadline = Instant::now() + PROBE_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Probe::TimedOut;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break None,
        }
    };
    let buf = reader.join().unwrap_or_default();
    match status {
        Some(status) if status.success() => parse(&buf).map_or(Probe::Failed, Probe::Info),
        _ => Probe::Failed,
    }
}

/// Parses ffprobe output; rejects documents that carry neither format nor streams.
fn parse(bytes: &[u8]) -> Option<Value> {
    let json: Value = serde_json::from_slice(bytes).ok()?;
    let has_format = json.get("format").is_some_and(Value::is_object);
    let has_streams = json
        .get("streams")
        .and_then(Value::as_array)
        .is_some_and(|s| !s.is_empty());
    (has_format || has_streams).then_some(json)
}

/// Builds the table rows from ffprobe's JSON. `file_size` (from the file system) wins
/// over the size ffprobe reports.
fn rows_from_json(json: &Value, file_size: Option<u64>) -> Vec<(&'static str, String)> {
    let format = json.get("format");
    let mut rows = Vec::new();

    let format_name = format
        .and_then(|f| text(f, "format_long_name").or_else(|| text(f, "format_name")))
        .unwrap_or_default();
    rows.push(("format", format_name));
    let duration = format.and_then(|f| number(f, "duration"));
    rows.push((
        "duration",
        duration.map(format_duration).unwrap_or_default(),
    ));
    let size = file_size.or_else(|| format.and_then(|f| number(f, "size")).map(|s| s as u64));
    rows.push(("size", size.map(human_size).unwrap_or_default()));
    let bitrate = format.and_then(|f| number(f, "bit_rate"));
    rows.push(("bitrate", bitrate.map(format_bitrate).unwrap_or_default()));

    if let Some(tags) = format.and_then(|f| f.get("tags")) {
        rows.push(("title", tag(tags, "title")));
        rows.push(("created", tag(tags, "creation_time")));
        rows.push(("encoder", tag(tags, "encoder")));
    }

    let streams = json.get("streams").and_then(Value::as_array);
    for stream in streams.into_iter().flatten() {
        let (key, value) = match text(stream, "codec_type").as_deref() {
            Some("video") if is_cover_art(stream) => ("cover", cover_line(stream)),
            Some("video") => ("video", video_line(stream)),
            Some("audio") => ("audio", audio_line(stream)),
            Some("subtitle") => ("subtitle", subtitle_line(stream)),
            _ => continue,
        };
        rows.push((key, value));
    }
    rows
}

fn is_cover_art(stream: &Value) -> bool {
    stream
        .pointer("/disposition/attached_pic")
        .and_then(Value::as_i64)
        == Some(1)
}

fn cover_line(stream: &Value) -> String {
    join([text(stream, "codec_name"), dimensions(stream)])
}

fn video_line(stream: &Value) -> String {
    let codec = text(stream, "codec_name").map(|codec| match text(stream, "profile") {
        Some(profile) => format!("{codec} ({profile})"),
        None => codec,
    });
    let fps = frame_rate(stream, "avg_frame_rate")
        .or_else(|| frame_rate(stream, "r_frame_rate"))
        .map(|fps| format!("{} fps", trim_float(fps)));
    let hdr = match text(stream, "color_transfer").as_deref() {
        Some("smpte2084") => Some("HDR10".to_string()),
        Some("arib-std-b67") => Some("HLG".to_string()),
        _ => None,
    };
    join([codec, dimensions(stream), fps, text(stream, "pix_fmt"), hdr])
}

fn audio_line(stream: &Value) -> String {
    let rate = number(stream, "sample_rate")
        .filter(|r| *r > 0.0)
        .map(|r| format!("{} kHz", trim_float(r / 1000.0)));
    let channels = text(stream, "channel_layout")
        .filter(|l| l != "unknown")
        .or_else(|| {
            number(stream, "channels")
                .filter(|c| *c > 0.0)
                .map(|c| format!("{c} ch"))
        });
    let bitrate = number(stream, "bit_rate")
        .filter(|b| *b > 0.0)
        .map(format_bitrate);
    join([
        text(stream, "codec_name"),
        rate,
        channels,
        bitrate,
        language(stream),
    ])
}

fn subtitle_line(stream: &Value) -> String {
    let title = stream
        .get("tags")
        .map(|t| tag(t, "title"))
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\""));
    join([text(stream, "codec_name"), language(stream), title])
}

fn language(stream: &Value) -> Option<String> {
    let lang = tag(stream.get("tags")?, "language");
    (!lang.is_empty() && lang != "und").then_some(lang)
}

fn dimensions(stream: &Value) -> Option<String> {
    let width = number(stream, "width")?;
    let height = number(stream, "height")?;
    (width > 0.0 && height > 0.0).then(|| format!("{width}x{height}"))
}

/// Joins the present parts with ", ".
fn join<const N: usize>(parts: [Option<String>; N]) -> String {
    parts
        .into_iter()
        .flatten()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A string field, skipping empty values.
fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A numeric field; ffprobe prints many numbers as strings.
fn number(value: &Value, key: &str) -> Option<f64> {
    match value.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

/// A tag value, looked up case-insensitively (Matroska writes upper-case keys).
fn tag(tags: &Value, name: &str) -> String {
    tags.as_object()
        .and_then(|map| {
            map.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .and_then(|(_, v)| v.as_str())
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// "60/1" or "30000/1001" to frames per second; "0/0" is unknown.
fn frame_rate(stream: &Value, key: &str) -> Option<f64> {
    let (num, den) = stream.get(key)?.as_str()?.split_once('/')?;
    let (num, den): (f64, f64) = (num.parse().ok()?, den.parse().ok()?);
    (num > 0.0 && den > 0.0).then(|| num / den)
}

/// 60 -> "60", 29.97003 -> "29.97", 44.1 -> "44.1".
fn trim_float(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn format_duration(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (hours, minutes, secs) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

fn format_bitrate(bits_per_second: f64) -> String {
    let kbps = bits_per_second / 1000.0;
    if kbps >= 1000.0 {
        format!("{} Mbps", trim_float(kbps / 1000.0))
    } else {
        format!("{} kbps", kbps.round())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "streams": [
        {"index": 0, "codec_name": "h264", "profile": "High", "codec_type": "video",
         "width": 1920, "height": 1080, "pix_fmt": "yuv420p",
         "color_transfer": "bt709", "r_frame_rate": "60/1", "avg_frame_rate": "60/1",
         "disposition": {"default": 1, "attached_pic": 0},
         "tags": {"language": "und", "handler_name": "VideoHandler"}},
        {"index": 1, "codec_name": "hevc", "profile": "Main 10", "codec_type": "video",
         "width": 3840, "height": 2160, "pix_fmt": "yuv420p10le",
         "color_transfer": "smpte2084", "r_frame_rate": "30000/1001",
         "avg_frame_rate": "30000/1001", "disposition": {"attached_pic": 0}},
        {"index": 2, "codec_name": "aac", "codec_type": "audio", "sample_rate": "48000",
         "channels": 2, "channel_layout": "stereo", "bit_rate": "160000",
         "tags": {"language": "eng"}},
        {"index": 3, "codec_name": "ac3", "codec_type": "audio", "sample_rate": "44100",
         "channels": 6, "bit_rate": "448000", "tags": {"language": "und"}},
        {"index": 4, "codec_name": "subrip", "codec_type": "subtitle",
         "tags": {"language": "eng", "title": "Full\u001b[31m subs"}},
        {"index": 5, "codec_name": "mjpeg", "codec_type": "video", "width": 500, "height": 500,
         "disposition": {"attached_pic": 1}},
        {"index": 6, "codec_name": "bin_data", "codec_type": "data"}
      ],
      "format": {
        "filename": "file:/tmp/clip.mkv", "format_name": "matroska,webm",
        "format_long_name": "Matroska / WebM", "duration": "3725.4", "size": "123456789",
        "bit_rate": "2650000",
        "tags": {"TITLE": "Demo clip", "ENCODER": "Lavf61.1.100",
                 "creation_time": "2026-10-07T14:53:47.000000Z"}
      }
    }"#;

    fn rows() -> Vec<(&'static str, String)> {
        let json: Value = serde_json::from_str(SAMPLE).unwrap();
        rows_from_json(&json, None)
    }

    fn row<'a>(rows: &'a [(&str, String)], key: &str, nth: usize) -> &'a str {
        rows.iter()
            .filter(|(k, _)| *k == key)
            .nth(nth)
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("no row {key}#{nth}"))
    }

    #[test]
    fn formats_container_rows() {
        let rows = rows();
        assert_eq!(row(&rows, "format", 0), "Matroska / WebM");
        assert_eq!(row(&rows, "duration", 0), "1:02:05");
        assert_eq!(row(&rows, "size", 0), "117.7 MiB");
        assert_eq!(row(&rows, "bitrate", 0), "2.65 Mbps");
        assert_eq!(row(&rows, "title", 0), "Demo clip");
        assert_eq!(row(&rows, "encoder", 0), "Lavf61.1.100");
        assert_eq!(row(&rows, "created", 0), "2026-10-07T14:53:47.000000Z");
    }

    #[test]
    fn formats_video_streams() {
        let rows = rows();
        assert_eq!(
            row(&rows, "video", 0),
            "h264 (High), 1920x1080, 60 fps, yuv420p"
        );
        assert_eq!(
            row(&rows, "video", 1),
            "hevc (Main 10), 3840x2160, 29.97 fps, yuv420p10le, HDR10"
        );
        assert_eq!(row(&rows, "cover", 0), "mjpeg, 500x500");
    }

    #[test]
    fn formats_audio_and_subtitle_streams() {
        let rows = rows();
        assert_eq!(row(&rows, "audio", 0), "aac, 48 kHz, stereo, 160 kbps, eng");
        assert_eq!(row(&rows, "audio", 1), "ac3, 44.1 kHz, 6 ch, 448 kbps");
        assert_eq!(
            row(&rows, "subtitle", 0),
            "subrip, eng, \"Full\u{1b}[31m subs\""
        );
        assert_eq!(rows.iter().filter(|(k, _)| *k == "data").count(), 0);
    }

    #[test]
    fn rendered_table_is_sanitized_and_skips_empty_rows() {
        let json: Value = serde_json::from_str(SAMPLE).unwrap();
        let mut out = Vec::new();
        kv::write_table(
            &mut out,
            "clip.mkv",
            &rows_from_json(&json, Some(10)),
            false,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains('\x1b'), "{text:?}");
        assert!(text.contains("^[[31m subs"));
        assert!(text.contains("size      10 B"), "{text}");
        assert!(text.starts_with("clip.mkv\n"));
    }

    #[test]
    fn minimal_json_still_renders() {
        let json: Value = serde_json::from_str(r#"{"format": {"duration": "59.6"}}"#).unwrap();
        let rows = rows_from_json(&json, None);
        assert_eq!(row(&rows, "duration", 0), "1:00");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse(b"not json").is_none());
        assert!(parse(b"{}").is_none());
        assert!(parse(br#"{"streams": [], "format": {}}"#).is_some());
    }

    #[test]
    fn unit_helpers() {
        assert_eq!(format_duration(0.4), "0:00");
        assert_eq!(format_duration(65.0), "1:05");
        assert_eq!(format_bitrate(160_000.0), "160 kbps");
        assert_eq!(trim_float(24.0), "24");
        assert_eq!(trim_float(23.976), "23.98");
    }
}
