//! Audio info view: tags and properties (lofty) plus a peak waveform (symphonia).

use std::fs::File;
use std::io::{self, Cursor, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use lofty::config::ParseOptions;
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::picture::PictureInformation;
use lofty::probe::Probe;
use lofty::tag::{Accessor, ItemKey, Tag};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

use crate::input::{Input, InputKind, Source};
use crate::kv;
use crate::util::human_size;
use crate::waveform;

/// Non-seekable input (stdin, pipes) is read into memory up to this size.
pub const MAX_STREAM_BYTES: u64 = 64 * 1024 * 1024;

/// Decoding stops after this much wall-clock time; the waveform then covers only the
/// part decoded so far and says so.
const DECODE_BUDGET: Duration = Duration::from_millis(1000);

/// Peaks are collected per block of this many milliseconds, then resampled to columns.
const BLOCK_MS: u64 = 10;

/// Extensions of audio formats lofty reads that `infer` does not recognise by content.
const EXTRA_EXTENSIONS: &[&str] = &["wv", "ape", "mpc", "spx", "aac"];

/// True for audio files that content sniffing misses (WavPack, Monkey's Audio, ...).
pub fn has_audio_extension(file_name: Option<&str>) -> bool {
    file_name
        .and_then(|n| Path::new(n).extension())
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTRA_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

enum Media {
    File(std::path::PathBuf),
    Memory(Vec<u8>),
}

impl Media {
    fn source(&self) -> io::Result<Box<dyn MediaSource + '_>> {
        Ok(match self {
            Media::File(path) => Box::new(File::open(path)?),
            Media::Memory(data) => Box::new(Cursor::new(data.as_slice())),
        })
    }
}

pub fn render<W: Write>(
    out: &mut W,
    source: &Source,
    input: &mut Input,
    mime: &str,
    colors: bool,
    utf8: bool,
) -> io::Result<()> {
    let (media, size) = match (source, input.kind) {
        (Source::Path(path), InputKind::Regular { size }) => (Media::File(path.clone()), size),
        _ => {
            let mut data = Vec::new();
            (&mut input.reader)
                .take(MAX_STREAM_BYTES + 1)
                .read_to_end(&mut data)?;
            if data.len() as u64 > MAX_STREAM_BYTES {
                return Err(io::Error::other(format!(
                    "audio input over {}; audio info needs a file (use --raw for bytes)",
                    human_size(MAX_STREAM_BYTES)
                )));
            }
            let size = data.len() as u64;
            (Media::Memory(data), size)
        }
    };
    let extension = source.file_name().and_then(|n| {
        Path::new(&n)
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
    });

    let tagged = read_tags(&media);
    let decoded = decode_peaks(&media, extension.as_deref());

    let mut rows: Vec<(&str, String)> = Vec::new();
    match &tagged {
        Ok(file) => info_rows(&mut rows, file, decoded.as_ref(), size),
        Err(err) => {
            rows.push(("format", mime.to_string()));
            if let Some(d) = &decoded {
                rows.push(("codec", d.codec.clone()));
            }
            rows.push(("size", human_size(size)));
            let err = err.find(" (os error ").map_or(err.as_str(), |i| &err[..i]);
            rows.push(("tags", format!("unreadable ({err})")));
        }
    }
    let title = match source {
        Source::Stdin => "-".to_string(),
        Source::Path(p) => p.file_name().map_or_else(
            || p.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        ),
    };
    kv::write_table(out, &title, &rows, colors)?;

    if let Some(decoded) = decoded
        && !decoded.peaks.is_empty()
    {
        let mut peaks = decoded.peaks;
        waveform::normalize(&mut peaks);
        writeln!(out)?;
        let mut width = waveform::terminal_width();
        // A partial waveform takes only its share of the width, so time stays to scale.
        let total = tagged
            .as_ref()
            .map(|f| f.properties().duration())
            .unwrap_or_default();
        if let Some(covered) = decoded.truncated_at
            && covered < total
        {
            let share = covered.as_secs_f64() / total.as_secs_f64();
            width = ((width as f64 * share).ceil() as usize).clamp(1, width);
        }
        waveform::write(out, &peaks, width, utf8, colors)?;
        if let Some(covered) = decoded.truncated_at {
            let (dim, reset) = if colors {
                ("\x1b[2m", "\x1b[0m")
            } else {
                ("", "")
            };
            writeln!(
                out,
                "{dim}waveform covers the first {} only{reset}",
                format_duration(covered)
            )?;
        }
    }
    Ok(())
}

fn read_tags(media: &Media) -> Result<TaggedFile, String> {
    let options = ParseOptions::new().read_cover_art(true);
    let result = match media {
        Media::File(path) => Probe::open(path)
            .map_err(|e| e.to_string())?
            .options(options)
            .guess_file_type()
            .map_err(|e| e.to_string())?
            .read(),
        Media::Memory(data) => Probe::new(Cursor::new(data.as_slice()))
            .options(options)
            .guess_file_type()
            .map_err(|e| e.to_string())?
            .read(),
    };
    result.map_err(|e| e.to_string())
}

fn info_rows(
    rows: &mut Vec<(&'static str, String)>,
    file: &TaggedFile,
    decoded: Option<&Decoded>,
    size: u64,
) {
    let tag = file.primary_tag().or_else(|| file.first_tag());
    let text = |key: ItemKey| -> String {
        tag.and_then(|t| t.get_string(key))
            .map(first_line)
            .unwrap_or_default()
    };
    rows.push(("title", text(ItemKey::TrackTitle)));
    rows.push(("artist", text(ItemKey::TrackArtist)));
    rows.push(("album", text(ItemKey::AlbumTitle)));
    rows.push(("album artist", text(ItemKey::AlbumArtist)));
    rows.push((
        "track",
        number_of(tag.and_then(Tag::track), tag.and_then(Tag::track_total)),
    ));
    rows.push((
        "disc",
        number_of(tag.and_then(Tag::disk), tag.and_then(Tag::disk_total)),
    ));
    let date = [ItemKey::RecordingDate, ItemKey::Year, ItemKey::ReleaseDate]
        .into_iter()
        .map(text)
        .find(|d| !d.is_empty())
        .or_else(|| tag.and_then(Tag::date).map(|d| d.to_string()))
        .unwrap_or_default();
    rows.push(("date", date));
    rows.push(("genre", text(ItemKey::Genre)));
    rows.push(("comment", text(ItemKey::Comment)));

    let props = file.properties();
    let duration = props.duration();
    rows.push((
        "duration",
        if duration.is_zero() {
            String::new()
        } else {
            format_duration(duration)
        },
    ));
    rows.push(("format", format_name(file.file_type()).to_string()));
    if let Some(d) = decoded {
        rows.push(("codec", d.codec.clone()));
    }
    rows.push((
        "bitrate",
        props
            .audio_bitrate()
            .or(props.overall_bitrate())
            .filter(|&b| b > 0)
            .map(|b| format!("{b} kbps"))
            .unwrap_or_default(),
    ));
    rows.push((
        "sample rate",
        props
            .sample_rate()
            .filter(|&r| r > 0)
            .map(format_khz)
            .unwrap_or_default(),
    ));
    rows.push((
        "channels",
        props.channels().map(channel_name).unwrap_or_default(),
    ));
    rows.push((
        "bit depth",
        props
            .bit_depth()
            .filter(|&b| b > 0)
            .map(|b| format!("{b} bit"))
            .unwrap_or_default(),
    ));
    rows.push(("size", human_size(size)));
    rows.push(("cover art", cover_art(file)));
}

/// "3/12", "3" or "".
fn number_of(n: Option<u32>, total: Option<u32>) -> String {
    match (n, total) {
        (Some(n), Some(t)) => format!("{n}/{t}"),
        (Some(n), None) => n.to_string(),
        (None, Some(t)) => format!("?/{t}"),
        (None, None) => String::new(),
    }
}

/// The first line of a multi-line value, marked when more lines follow.
fn first_line(value: &str) -> String {
    let value = value.trim();
    match value.split_once(['\n', '\r']) {
        Some((first, _)) => format!("{} …", first.trim_end()),
        None => value.to_string(),
    }
}

fn cover_art(file: &TaggedFile) -> String {
    let Some(picture) = file.tags().iter().find_map(|t| t.pictures().first()) else {
        return String::new();
    };
    let kind = picture
        .mime_type()
        .map(|m| m.ext().unwrap_or(m.as_str()).to_string())
        .unwrap_or_else(|| "unknown type".into());
    match PictureInformation::from_picture(picture) {
        Ok(info) if info.width > 0 && info.height > 0 => {
            format!("yes ({kind}, {}x{})", info.width, info.height)
        }
        _ => format!("yes ({kind})"),
    }
}

fn format_name(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Aac => "AAC (ADTS)",
        FileType::Aiff => "AIFF",
        FileType::Ape => "Monkey's Audio",
        FileType::Flac => "FLAC",
        FileType::Mpeg => "MPEG audio",
        FileType::Mp4 => "MP4 audio",
        FileType::Mpc => "Musepack",
        FileType::Opus => "Ogg Opus",
        FileType::Vorbis => "Ogg Vorbis",
        FileType::Speex => "Ogg Speex",
        FileType::Wav => "WAV",
        FileType::WavPack => "WavPack",
        _ => "unknown",
    }
}

fn format_khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{} kHz", rate / 1000)
    } else {
        format!("{:.1} kHz", f64::from(rate) / 1000.0)
    }
}

fn channel_name(channels: u8) -> String {
    match channels {
        1 => "1 (mono)".into(),
        2 => "2 (stereo)".into(),
        n => n.to_string(),
    }
}

/// `m:ss` below an hour, `h:mm:ss` from an hour on. Rounds to the nearest second.
pub fn format_duration(duration: Duration) -> String {
    let total = (duration.as_millis() + 500) / 1000;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

struct Decoded {
    /// Peak absolute amplitude per block, across all channels.
    peaks: Vec<f32>,
    /// Codec name as reported by the decoder, e.g. "MPEG Audio Layer 3".
    codec: String,
    /// Set when decoding stopped early: how much audio the peaks cover.
    truncated_at: Option<Duration>,
}

/// Decodes the default audio track and collects peaks. Any failure means no waveform.
fn decode_peaks(media: &Media, extension: Option<&str>) -> Option<Decoded> {
    let started = Instant::now();
    let mss = MediaSourceStream::new(media.source().ok()?, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = extension {
        hint.with_extension(ext);
    }
    let meta_opts = MetadataOptions::default();
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), meta_opts)
        .ok()?;
    let track = format.default_track(TrackType::Audio)?;
    let track_id = track.id;
    let params = track.codec_params.as_ref()?.audio()?.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .ok()?;
    let codec = decoder.codec_info().long_name.to_string();

    let mut peaks = Vec::new();
    let mut block_len = 0usize;
    let mut block_peak = 0f32;
    let mut frames_done = 0u64;
    let mut rate = params.sample_rate.unwrap_or(0);
    let mut samples: Vec<f32> = Vec::new();
    let mut truncated_at = None;
    let mut errors = 0u32;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) if errors < 100 => {
                errors += 1;
                continue;
            }
            Err(_) => break,
        };
        if packet.track_id != track_id {
            continue;
        }
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            Err(SymphoniaError::DecodeError(_) | SymphoniaError::IoError(_)) => {
                errors += 1;
                if errors > 1000 {
                    break;
                }
                continue;
            }
            Err(_) => break,
        };
        let channels = buffer.spec().channels().count().max(1);
        if rate == 0 {
            rate = buffer.spec().rate();
        }
        let block_frames = (u64::from(rate.max(1)) * BLOCK_MS / 1000).max(1) as usize;
        buffer.copy_to_vec_interleaved(&mut samples);
        for frame in samples.chunks_exact(channels) {
            for &s in frame {
                let a = s.abs();
                if a > block_peak && a.is_finite() {
                    block_peak = a;
                }
            }
            block_len += 1;
            if block_len == block_frames {
                peaks.push(block_peak);
                block_len = 0;
                block_peak = 0.0;
            }
        }
        frames_done += buffer.frames() as u64;
        if started.elapsed() > DECODE_BUDGET {
            truncated_at = Some(Duration::from_millis(
                frames_done * 1000 / u64::from(rate.max(1)),
            ));
            break;
        }
    }
    if block_len > 0 {
        peaks.push(block_peak);
    }
    Some(Decoded {
        peaks,
        codec,
        truncated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0:00");
        assert_eq!(format_duration(Duration::from_millis(59_499)), "0:59");
        assert_eq!(format_duration(Duration::from_millis(59_500)), "1:00");
        assert_eq!(format_duration(Duration::from_secs(3 * 60 + 7)), "3:07");
        assert_eq!(format_duration(Duration::from_secs(3600)), "1:00:00");
        assert_eq!(
            format_duration(Duration::from_secs(2 * 3600 + 5 * 60 + 9)),
            "2:05:09"
        );
    }

    #[test]
    fn formats_numbers_and_rates() {
        assert_eq!(number_of(Some(3), Some(12)), "3/12");
        assert_eq!(number_of(Some(3), None), "3");
        assert_eq!(number_of(None, None), "");
        assert_eq!(format_khz(44_100), "44.1 kHz");
        assert_eq!(format_khz(48_000), "48 kHz");
    }

    #[test]
    fn keeps_first_line_only() {
        assert_eq!(first_line("one\ntwo"), "one …");
        assert_eq!(first_line("  single  "), "single");
    }
}
