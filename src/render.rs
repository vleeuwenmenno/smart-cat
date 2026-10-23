//! Per-file dispatch: cat-compatible output or rendering.

use std::io::{self, BufRead, BufReader, BufWriter, IsTerminal, Read, Write};
use std::path::Path;

use syntect::easy::HighlightLines;

use crate::archive;
use crate::audio;
use crate::binary;
use crate::cat::{self, CatOptions, CatState};
use crate::cli::Cli;
use crate::config::{AnsiMode, Config, Directories, FileSettings, ListStyle, PagingMode, Renderer};
use crate::decompress;
use crate::detect::{self, Kind, SNIFF_BYTES};
use crate::diagram;
use crate::fastcopy;
use crate::header;
use crate::hexdump;
use crate::highlight::{self, Assets};
use crate::input::{self, Input, InputKind, Source};
use crate::json;
use crate::linestyle;
use crate::listing;
use crate::markdown;
use crate::notebook;
use crate::office;
use crate::pager;
use crate::subtitles;
use crate::table;
use crate::term::{self, ColorDepth, query};
use crate::util::{TrackedWriter, human_size, io_message, read_line_capped};
use crate::video;

/// Lines longer than this are shown unhighlighted from that point on.
const MAX_RENDER_LINE: usize = 1024 * 1024;
const RESET: &[u8] = b"\x1b[0m";

/// Prints the files. `strict` (invoked as `cat` without `[cat_mode] render = true`)
/// means no rendering on a terminal either, unless `--force-render` asks for it, and
/// never a pager, a directory listing or a listing for no arguments.
pub fn run(cli: &Cli, config: &Config, strict: bool) -> i32 {
    let strict_config;
    let config = if strict {
        let mut cat = config.clone();
        cat.defaults.paging = PagingMode::Never;
        cat.defaults.no_args = crate::config::NoArgs::Stdin;
        cat.defaults.directories = Directories::Error;
        strict_config = cat;
        &strict_config
    } else {
        config
    };
    let stdout = io::stdout();
    let stdout_tty = stdout.is_terminal();
    let render =
        !cli.raw && (cli.force_render || (stdout_tty && !strict)) && !cli.wants_cat_visualisation();
    let out_identity = input::stdout_identity();

    let mut ctx = Context::new(cli, config, stdout_tty);

    let mut sources = Source::from_args(&cli.files);
    if cli.files.is_empty()
        && listing::no_args_action(io::stdin().is_terminal(), render, config.defaults.no_args)
            == listing::NoArgsAction::ListCurrentDir
    {
        sources = vec![Source::Path(".".into())];
    }
    if render && config.defaults.directories == Directories::List {
        ctx.dir_args = sources
            .iter()
            .filter(|s| matches!(s, Source::Path(p) if p.is_dir()))
            .count();
    }
    ctx.show_headers =
        render && stdout_tty && sources.len() > 1 && !cli.plain && config.defaults.headers;

    // Decide on paging up front: only when every file is text-like (no graphics, no media).
    let mode = if strict {
        PagingMode::Never
    } else {
        cli.paging.unwrap_or(config.defaults.paging)
    };
    let terminal = if render && mode != PagingMode::Never {
        pager::Terminal::detect()
    } else {
        pager::Terminal::default()
    };
    let decision = pager::decide(mode, &terminal, render, || all_pageable(&sources, &mut ctx));
    let mut output = pager::Output::new(stdout.lock(), pager::plan(decision));
    let paged = output.is_paged();
    let status = {
        let mut out = TrackedWriter::new(BufWriter::with_capacity(64 * 1024, &mut output));
        process_all(&sources, render, out_identity, paged, &mut ctx, &mut out)
    };
    match output.finish() {
        Err(err) if paged && err.kind() == io::ErrorKind::BrokenPipe => status,
        Err(err) => {
            crate::report!("write error: {}", io_message(&err));
            1
        }
        Ok(()) => status,
    }
}

/// Whether an error is the pager having quit early: stop quietly, not a failure.
fn pager_quit(paged: bool, err: &io::Error) -> bool {
    paged && err.kind() == io::ErrorKind::BrokenPipe
}

fn process_all<W: Write>(
    sources: &[Source],
    render: bool,
    out_identity: Option<(u64, u64)>,
    paged: bool,
    ctx: &mut Context,
    out: &mut TrackedWriter<W>,
) -> i32 {
    let mut cat_state = CatState::default();
    let mut status = 0;
    for source in sources {
        let result = process(source, render, out_identity, ctx, out, &mut cat_state);
        if let Err(err) = result {
            if out.failed {
                if pager_quit(paged, &err) {
                    return 0;
                }
                crate::report!("write error: {}", io_message(&err));
                return 1;
            }
            let _ = out.flush();
            crate::report!("{}: {}", source.display_name(), io_message(&err));
            status = 1;
        }
    }
    if let Err(err) = cat::finish(out, &mut cat_state).and_then(|()| out.flush()) {
        if pager_quit(paged, &err) {
            return 0;
        }
        crate::report!("write error: {}", io_message(&err));
        return 1;
    }
    status
}

/// True when every source is a file or directory that renders as plain text (or a hex
/// dump), so a pager can show it. Standard input cannot be sniffed ahead of time.
fn all_pageable(sources: &[Source], ctx: &mut Context) -> bool {
    sources.iter().all(|source| source_pageable(source, ctx))
}

fn source_pageable(source: &Source, ctx: &mut Context) -> bool {
    let Source::Path(path) = source else {
        return false;
    };
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.is_dir() {
        if ctx.config.defaults.directories != Directories::List {
            return false;
        }
        // Tiles with kitty or sixel thumbnails must never reach a pager.
        let (style, screen) = list_style(ctx);
        return !listing::draws_graphics(style, list_thumbnails(ctx), screen.map(|s| s.method));
    }
    if !meta.is_file() {
        return false;
    }
    let mut head = Vec::new();
    let sniffed = std::fs::File::open(path)
        .and_then(|file| file.take(SNIFF_BYTES as u64).read_to_end(&mut head));
    if sniffed.is_err() {
        return false;
    }
    // Compressed files are decoded later and may hold anything, images included.
    if let Some(format) = decompress::detect(&head) {
        // A compressed tarball gets the archive listing, which pages like text.
        let name = source.file_name();
        if !decompress::is_tarball(name.as_deref(), &head, format) {
            return false;
        }
        let mime = detect::detect(&head).mime;
        let renderer = ctx.config.settings_for(name.as_deref(), &mime).renderer;
        return matches!(renderer, Renderer::Auto | Renderer::Archive | Renderer::Hex);
    }
    let detected = detect::detect(&head);
    let file_name = source.file_name();
    let settings = ctx
        .config
        .settings_for(file_name.as_deref(), &detected.mime);
    // A font's specimen is an image; only the table alone can be paged.
    if settings.renderer == Renderer::Font
        || (settings.renderer == Renderer::Auto && crate::font::wants(&head))
    {
        return font_screen(ctx).is_none();
    }
    let language = ctx.cli.language.as_deref().or(settings.language.as_deref());
    let as_markdown = match settings.renderer {
        Renderer::Markdown => true,
        Renderer::Auto => markdown::is_markdown(file_name.as_deref(), language),
        _ => false,
    };
    // Markdown may draw inline images, and graphics escapes must never reach a pager.
    if as_markdown && markdown_has_images(path) {
        return false;
    }
    // Notebooks draw image outputs; the same holds for a pager.
    let as_notebook = match settings.renderer {
        Renderer::Notebook => true,
        Renderer::Auto => notebook::is_notebook(file_name.as_deref(), language),
        _ => false,
    };
    if as_notebook && notebook_has_images(path) {
        return false;
    }
    // Diagrams are drawn as images.
    if settings.renderer == Renderer::Auto
        && detected.kind == Kind::Text
        && diagram::detect(file_name.as_deref(), language).is_some()
    {
        return false;
    }
    pager::renderer_is_pageable(
        settings.renderer,
        &detected.kind,
        &detected.mime,
        crate::image::wants(&detected.mime, file_name.as_deref(), &head),
        is_audio(&detected, file_name.as_deref()),
    )
}

/// Whether a markdown file references an image (`![`) or has a fenced diagram block,
/// read up to 4 MiB.
fn markdown_has_images(path: &std::path::Path) -> bool {
    let mut text = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(4 << 20).read_to_end(&mut text));
    read.is_err() || text.windows(2).any(|w| w == b"![") || diagram::markdown_has_diagrams(&text)
}

/// Whether a notebook may draw images (an image output or attachment), read up to
/// the notebook size limit.
fn notebook_has_images(path: &std::path::Path) -> bool {
    let mut data = Vec::new();
    let read = std::fs::File::open(path)
        .and_then(|f| f.take(notebook::MAX_BYTES + 1).read_to_end(&mut data));
    read.is_err() || notebook::may_draw_images(&data)
}

/// The rule before a file when several are rendered, with a blank line between files.
fn write_header<W: Write>(source: &Source, ctx: &mut Context, out: &mut W) -> io::Result<()> {
    if ctx.printed > 0 {
        out.write_all(b"\n")?;
    }
    ctx.printed += 1;
    let name = match source {
        Source::Stdin => "-".to_string(),
        Source::Path(path) => path.to_string_lossy().into_owned(),
    };
    let width = listing::terminal_width();
    out.write_all(header::line(&name, width, ctx.colors, ctx.utf8).as_bytes())
}

fn process<W: Write>(
    source: &Source,
    render: bool,
    out_identity: Option<(u64, u64)>,
    ctx: &mut Context,
    out: &mut TrackedWriter<W>,
    cat_state: &mut CatState,
) -> io::Result<()> {
    if let Source::Path(path) = source
        && render
        && ctx.config.defaults.directories == Directories::List
        && path.is_dir()
    {
        return list_directory(path, ctx, out);
    }
    let mut input = input::open(source)?;
    if input::copies_onto_itself(source, &input, out_identity) {
        return Err(io::Error::other("input file is output file"));
    }

    if !render || input.kind == InputKind::Device {
        if ctx.cat_options.is_passthrough() {
            return fastcopy::copy(&mut input, out);
        }
        return write_cat(&mut input, out, &ctx.cat_options, cat_state);
    }
    if ctx.show_headers {
        write_header(source, ctx, out)?;
    }

    let mut head = sniff(&mut input)?;
    let mut detected = detect::detect(&head);
    let mut file_name = source.file_name();
    let mut settings = ctx
        .config
        .settings_for(file_name.as_deref(), &detected.mime);

    // Single-stream .gz/.xz/.zst/.bz2: decode, then treat the inside like any file.
    let mut truncated = None;
    if settings.renderer != Renderer::Raw
        && let Some(format) = decompress::detect(&head)
        && !decompress::is_tarball(file_name.as_deref(), &head, format)
    {
        let outer = std::mem::replace(&mut input.reader, BufReader::new(Box::new(io::empty())));
        let (reader, hit) = decompress::open(
            format,
            Box::new(outer),
            ctx.config.defaults.max_decompressed_bytes,
        )?;
        input.reader = BufReader::with_capacity(64 * 1024, reader);
        input.kind = InputKind::Stream;
        input.identity = None;
        input.fd = None;
        truncated = Some(hit);
        file_name = file_name.map(|name| decompress::inner_name(&name));
        head = sniff(&mut input)?;
        detected = detect::detect(&head);
        settings = ctx
            .config
            .settings_for(file_name.as_deref(), &detected.mime);
    }
    ctx.text_kind = text_kind(
        settings.renderer,
        ctx.cli.language.as_deref().or(settings.language.as_deref()),
        file_name.as_deref(),
        &head,
    );

    // Archives (compressed tarballs included) are listed, not decoded as text.
    let archive_format = archive::detect(&head, file_name.as_deref());
    let is_video = settings.renderer == Renderer::Auto && detected.mime.starts_with("video/");
    let language = ctx.cli.language.as_deref().or(settings.language.as_deref());
    let as_markdown = match settings.renderer {
        Renderer::Markdown => true,
        Renderer::Auto => markdown::is_markdown(file_name.as_deref(), language),
        _ => false,
    };

    let delimited = table::delimiter_for(file_name.as_deref(), language);
    if matches!(settings.renderer, Renderer::Auto | Renderer::Code)
        && detected.kind == Kind::Text
        && json::is_json(file_name.as_deref(), language)
    {
        json::prettify_input(&mut input, &head)?;
    }

    let result = match settings.renderer {
        Renderer::Video => render_video(source, &input, out, ctx, &detected.mime),
        Renderer::Auto if is_video => render_video(source, &input, out, ctx, &detected.mime),
        Renderer::Raw => write_cat(&mut input, out, &ctx.cat_options, cat_state),
        Renderer::Audio => audio::render(
            out,
            source,
            &mut input,
            &detected.mime,
            ctx.colors,
            ctx.utf8,
            &mut || ctx.image_screen(),
        ),
        Renderer::Auto if is_audio(&detected, file_name.as_deref()) => audio::render(
            out,
            source,
            &mut input,
            &detected.mime,
            ctx.colors,
            ctx.utf8,
            &mut || ctx.image_screen(),
        ),
        Renderer::Document => {
            let format = office::wants(&detected.mime, file_name.as_deref(), &head);
            render_document(&mut input, out, ctx, &settings, format)
        }
        Renderer::Auto if office::wants(&detected.mime, file_name.as_deref(), &head).is_some() => {
            let format = office::wants(&detected.mime, file_name.as_deref(), &head);
            render_document(&mut input, out, ctx, &settings, format)
        }
        Renderer::Pdf => render_pdf(&mut input, out, ctx, source),
        Renderer::Auto if crate::pdf::wants(&detected.mime, &head) => {
            render_pdf(&mut input, out, ctx, source)
        }
        Renderer::Database => render_sqlite(&mut input, out, ctx, source),
        Renderer::Auto if matches!(source, Source::Path(_)) && crate::sqlite::wants(&head) => {
            render_sqlite(&mut input, out, ctx, source)
        }
        Renderer::Subtitles => {
            render_subtitles(&mut input, out, ctx, &settings, file_name.as_deref(), &head)
        }
        Renderer::Auto
            if subtitles::wants(
                file_name.as_deref(),
                ctx.cli.language.as_deref().or(settings.language.as_deref()),
                &head,
            )
            .is_some() =>
        {
            render_subtitles(&mut input, out, ctx, &settings, file_name.as_deref(), &head)
        }
        Renderer::Font => render_font(&mut input, out, ctx, source),
        Renderer::Auto if crate::font::wants(&head) => render_font(&mut input, out, ctx, source),
        Renderer::Image => render_image(&mut input, out, ctx, source, &head),
        Renderer::Auto if crate::image::wants(&detected.mime, file_name.as_deref(), &head) => {
            render_image(&mut input, out, ctx, source, &head)
        }
        Renderer::Certificate => render_cert(&mut input, out, ctx, &settings, file_name.as_deref()),
        Renderer::Auto if crate::cert::wants(file_name.as_deref(), &head) => {
            render_cert(&mut input, out, ctx, &settings, file_name.as_deref())
        }
        Renderer::Diagram => render_diagram(
            &mut input,
            out,
            ctx,
            &settings,
            file_name.as_deref(),
            source,
        ),
        Renderer::Auto
            if detected.kind == Kind::Text
                && diagram::detect(file_name.as_deref(), language).is_some() =>
        {
            render_diagram(
                &mut input,
                out,
                ctx,
                &settings,
                file_name.as_deref(),
                source,
            )
        }
        Renderer::Table => render_table(
            &mut input,
            out,
            ctx,
            &settings,
            &head,
            delimited.unwrap_or(table::Delimiter::Sniff),
        ),
        Renderer::Auto if detected.kind == Kind::Text && delimited.is_some() => render_table(
            &mut input,
            out,
            ctx,
            &settings,
            &head,
            delimited.unwrap_or(table::Delimiter::Sniff),
        ),
        Renderer::Archive => render_archive(&mut input, out, ctx, source, archive_format),
        Renderer::Auto if archive_format.is_some() => {
            render_archive(&mut input, out, ctx, source, archive_format)
        }
        Renderer::Binary => render_binary(&mut input, out, ctx, source),
        Renderer::Auto if detected.kind == Kind::Binary && binary::wants(&head) => {
            render_binary(&mut input, out, ctx, source)
        }
        Renderer::Hex => render_hex(&mut input, out, ctx),
        Renderer::Auto if detected.kind == Kind::Binary => {
            if crate::cert::eligible(file_name.as_deref()) {
                hide_private_keys(&mut input);
            }
            render_hex(&mut input, out, ctx)
        }
        Renderer::Notebook => {
            render_notebook(&mut input, out, ctx, &settings, file_name.as_deref())
        }
        Renderer::Auto if notebook::is_notebook(file_name.as_deref(), language) => {
            render_notebook(&mut input, out, ctx, &settings, file_name.as_deref())
        }
        _ if as_markdown => {
            let doc = (&settings, file_name.as_deref(), source);
            render_markdown(&mut input, out, ctx, doc, &head)
        }
        _ => {
            // A key file the certificate view did not claim (the BEGIN line is past
            // the sniffed head, or its label is malformed) still has its key hidden.
            if settings.renderer == Renderer::Auto && crate::cert::eligible(file_name.as_deref()) {
                hide_private_keys(&mut input);
            }
            render_text(&mut input, out, ctx, &settings, file_name.as_deref(), &head)
        }
    };
    if truncated.is_some_and(|hit| hit.get()) {
        out.flush()?;
        crate::report!(
            "{}: decompressed output stopped at {} (max_decompressed_bytes)",
            source.display_name(),
            human_size(ctx.config.defaults.max_decompressed_bytes)
        );
    }
    result
}

/// The first bytes of the input for type detection, left unconsumed in the reader.
fn sniff(input: &mut Input) -> io::Result<Vec<u8>> {
    let head = input.reader.fill_buf()?;
    Ok(head[..head.len().min(SNIFF_BYTES)].to_vec())
}

/// Whether a text file gets diff or log colouring: forced by the renderer setting or
/// `-l diff|patch|log`, by extension, or (diffs, and logs without a known extension)
/// by what the first lines look like.
pub fn text_kind(
    renderer: Renderer,
    language: Option<&str>,
    file_name: Option<&str>,
    head: &[u8],
) -> Option<linestyle::Kind> {
    use linestyle::Kind;
    match renderer {
        Renderer::Diff => return Some(Kind::Diff),
        Renderer::Log => return Some(Kind::Log),
        Renderer::Auto => {}
        _ => return None,
    }
    let ext = file_name
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase());
    let named = language
        .map(str::to_ascii_lowercase)
        .or(ext.clone())
        .unwrap_or_default();
    match named.as_str() {
        "diff" | "patch" => return Some(Kind::Diff),
        "log" => return Some(Kind::Log),
        _ => {}
    }
    if language.is_some() || markdown::is_markdown(file_name, language) {
        return None;
    }
    if linestyle::looks_like_diff(head) {
        return Some(Kind::Diff);
    }
    let unnamed = ext
        .as_deref()
        .is_none_or(|e| matches!(e, "txt" | "out" | "err"));
    (unnamed && linestyle::looks_like_log(head)).then_some(Kind::Log)
}

/// Hex dump of the first `[hexdump] max_bytes` bytes.
fn render_hex<W: Write>(input: &mut Input, out: &mut W, ctx: &Context) -> io::Result<()> {
    let total = match input.kind {
        InputKind::Regular { size } => Some(size),
        _ => None,
    };
    let opts = hexdump::Options {
        colors: ctx.colors,
        utf8: ctx.utf8,
        max_bytes: ctx.config.hexdump.max_bytes.max(16),
    };
    hexdump::render(out, &mut input.reader, total, &opts)
}

/// Summary table for executables and libraries, then a short hex dump.
fn render_binary<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &Context,
    source: &Source,
) -> io::Result<()> {
    let total = match input.kind {
        InputKind::Regular { size } => Some(size),
        _ => None,
    };
    let opts = binary::Options {
        name: &source.display_name(),
        colors: ctx.colors,
        utf8: ctx.utf8,
        hex_bytes: ctx.config.binary.hex_bytes.max(16),
        fallback_bytes: ctx.config.hexdump.max_bytes.max(16),
    };
    binary::render(out, &mut input.reader, total, &opts)
}

/// Video info view. ffprobe opens the file by path, so standard input and pipes only
/// get a notice on stderr (exit status 0, like the binary notice).
fn render_video<W: Write>(
    source: &Source,
    input: &Input,
    out: &mut W,
    ctx: &mut Context,
    mime: &str,
) -> io::Result<()> {
    let (Source::Path(path), InputKind::Regular { size }) = (source, input.kind) else {
        out.flush()?;
        crate::report!(
            "{}: video info needs a file (use --raw for bytes)",
            source.display_name()
        );
        return Ok(());
    };
    let name = source.file_name().unwrap_or_else(|| source.display_name());
    let colors = ctx.colors;
    video::render(out, path, &name, Some(size), mime, colors, &mut || {
        ctx.image_screen()
    })
}

/// Replaces the input with one whose private key blocks are hidden (see
/// `cert::redact`). Used wherever `auto` or `cert` show a key file as text or hex.
fn hide_private_keys(input: &mut Input) {
    let rest = std::mem::replace(&mut input.reader, BufReader::new(Box::new(io::empty())));
    input.reader = BufReader::with_capacity(
        64 * 1024,
        Box::new(crate::cert::redact::Redacting::new(rest)),
    );
    input.fd = None;
}

/// Certificate and key summaries. Files that hold nothing recognisable (or are over
/// `cert::MAX_BYTES`) are shown as text or a hex dump instead, without an error, and
/// with any private key body hidden.
fn render_cert<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    settings: &FileSettings,
    file_name: Option<&str>,
) -> io::Result<()> {
    let mut data = Vec::new();
    (&mut input.reader)
        .take(crate::cert::MAX_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 <= crate::cert::MAX_BYTES {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let opts = crate::cert::Options {
            colors: ctx.colors,
            utf8: ctx.utf8,
            tab_width: settings.tab_width,
            now,
        };
        let mut rendered = Vec::new();
        if crate::cert::render(&mut rendered, &data, file_name, &opts)? {
            return out.write_all(&rendered);
        }
    }
    let binary = crate::cert::is_binary(&data[..data.len().min(SNIFF_BYTES)]);
    let rest = std::mem::replace(&mut input.reader, BufReader::new(Box::new(io::empty())));
    input.reader = BufReader::new(Box::new(io::Cursor::new(data).chain(rest)));
    // The fallback views never show a private key either.
    hide_private_keys(input);
    if binary {
        render_hex(input, out, ctx)
    } else {
        let head = sniff(input)?;
        render_text(input, out, ctx, settings, file_name, &head)
    }
}

fn render_table<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &Context,
    settings: &FileSettings,
    head: &[u8],
    delimiter: table::Delimiter,
) -> io::Result<()> {
    let cli = ctx.cli;
    let opts = table::Options {
        width: listing::terminal_width(),
        colors: ctx.colors,
        utf8: ctx.utf8,
        line_numbers: !cli.plain && (cli.number || cli.number_nonblank || settings.line_numbers),
        max_rows: ctx.config.table.max_rows,
    };
    table::render(&mut input.reader, head, delimiter, &opts, out)
}

fn list_directory<W: Write>(
    path: &std::path::Path,
    ctx: &mut Context,
    out: &mut TrackedWriter<W>,
) -> io::Result<()> {
    let (style, screen) = list_style(ctx);
    let list = &ctx.config.list;
    let opts = listing::Options {
        all: ctx.cli.all || list.show_hidden,
        style,
        colors: ctx.colors,
        width: listing::terminal_width(),
        tiles: listing::TileOptions {
            size: ctx
                .cli
                .tile_size
                .unwrap_or((list.tile_width, list.tile_height)),
            thumbnails: list_thumbnails(ctx),
            icons: list.icons,
            max_thumbnails: list.max_thumbnails,
            cache: list.cache,
            utf8: ctx.utf8,
            screen,
        },
    };
    // Build first so an unreadable directory prints no header, only the error.
    let mut buf = Vec::new();
    listing::list(path, &opts, &mut buf)?;
    if ctx.dir_args > 1 {
        if ctx.listed > 0 {
            out.write_all(b"\n")?;
        }
        writeln!(out, "{}:", listing::safe_name(&path.to_string_lossy()))?;
    }
    ctx.listed += 1;
    ctx.printed += 1;
    out.write_all(&buf)
}

/// The listing style for this run (`--long`, then `--style`, then the config), with
/// `auto` resolved, and the image screen when the tiles view is used.
fn list_style(ctx: &mut Context) -> (ListStyle, Option<crate::image::Screen>) {
    let requested = if ctx.cli.long {
        ListStyle::Long
    } else {
        ctx.cli.style.unwrap_or(ctx.config.list.style)
    };
    let basic = term::is_basic_terminal();
    let screen = match requested {
        ListStyle::Tiles => Some(ctx.image_screen()),
        ListStyle::Auto if ctx.stdout_tty && !basic => Some(ctx.image_screen()),
        _ => None,
    };
    let method = screen.as_ref().map(|s| s.method);
    let style = listing::resolve_style(requested, ctx.stdout_tty, basic, method);
    (style, screen.filter(|_| style == ListStyle::Tiles))
}

fn list_thumbnails(ctx: &Context) -> bool {
    !ctx.cli.no_thumbnails && ctx.config.list.thumbnails
}

/// Files that get the audio view with the `auto` renderer: an `audio/*` MIME type, or
/// an unrecognised binary file with an audio extension that sniffing misses (WavPack).
pub fn is_audio(detected: &detect::Detected, file_name: Option<&str>) -> bool {
    detected.mime.starts_with("audio/")
        || (detected.kind == Kind::Binary && audio::has_audio_extension(file_name))
}

/// Renders a markdown file. Files over `max_highlight_bytes` are shown as text instead.
fn render_markdown<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    (settings, file_name, source): (&FileSettings, Option<&str>, &Source),
    head: &[u8],
) -> io::Result<()> {
    let limit = ctx.config.defaults.max_highlight_bytes;
    if matches!(input.kind, InputKind::Regular { size } if size > limit) {
        return render_text(input, out, ctx, settings, file_name, head);
    }
    let mut data = Vec::new();
    (&mut input.reader).take(limit + 1).read_to_end(&mut data)?;
    if data.len() as u64 > limit {
        // A stream too large to hold: put back what was read and show it as text.
        let rest = std::mem::replace(&mut input.reader, BufReader::new(Box::new(io::empty())));
        input.reader = BufReader::new(Box::new(io::Cursor::new(data).chain(rest)));
        return render_text(input, out, ctx, settings, file_name, head);
    }
    let text = String::from_utf8_lossy(&data);

    let opts = markdown_options(ctx, settings, &text);
    // Both callbacks need the context; they never run at the same time.
    let ctx = std::cell::RefCell::new(ctx);
    let mut highlight = |lang: &str, code: &str| -> Option<Vec<Vec<u8>>> {
        highlight_block(&mut ctx.borrow_mut(), settings, lang, code)
    };
    // Image paths are relative to the document's directory; standard input has none,
    // so its images keep their placeholders.
    let base = match source {
        Source::Path(path) => Some(path.parent().map(Path::to_path_buf).unwrap_or_default()),
        Source::Stdin => None,
    };
    let mut images = |dest: &str, max_cols: u16| -> Option<crate::image::Drawn> {
        let path = markdown::local_image_path(base.as_deref()?, dest)?;
        let screen = ctx.borrow_mut().image_screen();
        markdown_image(&screen, &path, max_cols)
    };
    let name = source.display_name();
    let mut diagrams = |lang: &str, code: &str, max_cols: u16| {
        let kind = diagram::fence_kind(lang)?;
        let mut ctx = ctx.borrow_mut();
        if !ctx.colors {
            return None;
        }
        let screen = ctx.image_screen();
        drop(ctx);
        if screen.method == crate::image::Method::Ascii {
            return None;
        }
        let limits = (Some(max_cols), Some(MARKDOWN_DIAGRAM_ROWS));
        match draw_diagram(&screen, kind, code, limits) {
            Ok((drawn, _)) => Some((drawn, format!("{} diagram", kind.name()))),
            Err(message) => {
                crate::report!(
                    "{name}: cannot draw {} diagram: {}",
                    kind.name(),
                    sanitize_str(&message)
                );
                None
            }
        }
    };
    let rendered = markdown::render_with_diagrams(
        &text,
        &opts,
        &mut highlight,
        &mut images,
        Some(&mut diagrams),
    );
    out.write_all(&rendered)
}

/// Highlights a code block with the theme of `settings`: one encoded line per source
/// line, without newlines. `None` when the language or theme is unknown.
fn highlight_block(
    ctx: &mut Context,
    settings: &FileSettings,
    lang: &str,
    code: &str,
) -> Option<Vec<Vec<u8>>> {
    let theme_name = ctx.theme_name(settings);
    let assets = ctx.assets.get_or_insert_with(Assets::load);
    let syntax = assets.find_syntax(Some(lang), None, "")?;
    let theme = assets
        .theme(&theme_name)
        .or_else(|| assets.theme(&ctx.config.defaults.theme_dark))?;
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut lines = Vec::new();
    for line in syntect::util::LinesWithEndings::from(code) {
        let regions = highlighter.highlight_line(line, &assets.syntaxes).ok()?;
        let mut encoded = Vec::with_capacity(line.len() * 2);
        let mut column = 0;
        for (style, piece) in regions {
            let piece = piece.trim_end_matches(['\n', '\r']);
            if piece.is_empty() {
                continue;
            }
            encoded.extend_from_slice(highlight::style_sgr(style, ctx.depth).as_bytes());
            sanitize(
                &mut encoded,
                piece.as_bytes(),
                SgrMode::Escape,
                settings.tab_width,
                &mut column,
            );
        }
        encoded.extend_from_slice(RESET);
        lines.push(encoded);
    }
    Some(lines)
}

/// Jupyter notebooks: cells in order with their outputs. Notebooks over the size limit
/// and files that are not nbformat 4 fall back to the highlighted JSON view.
fn render_notebook<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    settings: &FileSettings,
    file_name: Option<&str>,
) -> io::Result<()> {
    let mut data = Vec::new();
    (&mut input.reader)
        .take(notebook::MAX_BYTES + 1)
        .read_to_end(&mut data)?;
    let name = file_name.unwrap_or("-").to_string();
    let parsed = if data.len() as u64 > notebook::MAX_BYTES {
        Err(format!("larger than {}", human_size(notebook::MAX_BYTES)))
    } else {
        notebook::parse(&data)
    };
    let nb = match parsed {
        Ok(nb) => nb,
        Err(reason) => {
            crate::report!("{name}: not a readable notebook ({reason}); showing JSON");
            let rest = std::mem::replace(&mut input.reader, BufReader::new(Box::new(io::empty())));
            input.reader = BufReader::new(Box::new(io::Cursor::new(data).chain(rest)));
            let head = sniff(input)?;
            return render_text(input, out, ctx, settings, Some("notebook.json"), &head);
        }
    };
    let has_code_spans = data.contains(&b'`');
    drop(data);

    let sgr_mode = match (ctx.config.defaults.ansi, ctx.colors) {
        (AnsiMode::Escape, _) => SgrMode::Escape,
        (AnsiMode::KeepSgr, true) => SgrMode::Keep,
        (AnsiMode::KeepSgr, false) => SgrMode::Strip,
    };
    let opts = notebook::Options {
        colors: ctx.colors,
        utf8: ctx.utf8,
        width: markdown::render_width(ctx.stdout_tty),
        tab_width: settings.tab_width,
        plain: ctx.cli.plain,
    };
    let md_opts = markdown_options(ctx, settings, if has_code_spans { "`" } else { "" });
    let tab_width = settings.tab_width;
    let ctx = std::cell::RefCell::new(ctx);
    let mut markdown_hook = |text: &str, images: notebook::MarkdownImages| -> Vec<u8> {
        let mut highlight =
            |lang: &str, code: &str| highlight_block(&mut ctx.borrow_mut(), settings, lang, code);
        markdown::render_with_images(text, &md_opts, &mut highlight, images)
    };
    let mut highlight =
        |lang: &str, code: &str| highlight_block(&mut ctx.borrow_mut(), settings, lang, code);
    let mut image = |data: Vec<u8>, svg: bool, cols: u16| {
        let screen = ctx.borrow_mut().image_screen();
        notebook_image(&screen, data, svg, cols)
    };
    let mut clean = |line: &str| {
        let mut encoded = Vec::with_capacity(line.len());
        let mut column = 0;
        sanitize(
            &mut encoded,
            line.as_bytes(),
            sgr_mode,
            tab_width,
            &mut column,
        );
        encoded
    };
    let mut hooks = notebook::Hooks {
        markdown: &mut markdown_hook,
        highlight: &mut highlight,
        image: &mut image,
        clean: &mut clean,
    };
    out.write_all(&notebook::render(&nb, &opts, &mut hooks))
}

/// Draws a notebook image output with kitty, sixel or blocks. ASCII output and
/// decoding errors keep the placeholder.
fn notebook_image(
    screen: &crate::image::Screen,
    data: Vec<u8>,
    svg: bool,
    max_cols: u16,
) -> Option<crate::image::Drawn> {
    if screen.method == crate::image::Method::Ascii {
        return None;
    }
    let opts = crate::image::ShowOptions {
        max_cols: Some(max_cols),
        max_rows: Some(notebook::IMAGE_ROWS),
        ..Default::default()
    };
    crate::image::draw_encoded(screen, data, svg, &opts).ok()
}

/// Only inline code uses the light/dark palette; code blocks ask for a theme
/// themselves. Documents without backticks never need the terminal's background.
fn markdown_options(ctx: &mut Context, settings: &FileSettings, text: &str) -> markdown::Options {
    let light = ctx.colors
        && ctx.depth != ColorDepth::Ansi16
        && text.contains('`')
        && ctx.markdown_light(settings);
    markdown::Options {
        width: markdown::render_width(ctx.stdout_tty),
        colors: ctx.colors,
        depth: ctx.depth,
        utf8: ctx.utf8,
        hyperlinks: ctx.colors && ctx.stdout_tty && !term::is_basic_terminal(),
        light,
        tab_width: settings.tab_width,
    }
}

/// Word, PowerPoint, Excel and OpenDocument files: converted to markdown, then drawn
/// by the markdown renderer. A dim header line names the type and the page, slide or
/// sheet count.
fn render_document<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    settings: &FileSettings,
    format: Option<office::Format>,
) -> io::Result<()> {
    let Some(format) = format else {
        return Err(io::Error::other(
            "cannot read document: not a recognised office document",
        ));
    };
    let size = match input.kind {
        InputKind::Regular { size } => Some(size),
        _ => None,
    };
    let data = office::read_input(input)?;
    let limits = office::Limits {
        max_rows: ctx.config.office.max_rows,
    };
    let converted = office::convert(format, &data, &limits)
        .map_err(|err| io::Error::other(format!("cannot read document: {err}")))?;
    drop(data);
    office::write_header(
        out,
        &office::header_text(format, &converted, size),
        ctx.colors,
        ctx.cli.plain,
    )?;
    let opts = markdown_options(ctx, settings, &converted.markdown);
    let rendered = markdown::render(&converted.markdown, &opts, &mut |_, _| None);
    out.write_all(&rendered)
}

/// Markdown images are at most this many rows high.
const MARKDOWN_IMAGE_ROWS: u16 = 15;
/// Diagrams in markdown documents are at most this many rows high.
const MARKDOWN_DIAGRAM_ROWS: u16 = 20;
/// Larger image files in markdown documents are left as placeholders.
const MARKDOWN_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

/// Draws a local image for a markdown document with kitty, sixel or blocks. ASCII
/// output, unreadable files and decoding errors keep the placeholder.
fn markdown_image(
    screen: &crate::image::Screen,
    path: &Path,
    max_cols: u16,
) -> Option<crate::image::Drawn> {
    if screen.method == crate::image::Method::Ascii {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut data = Vec::new();
    file.take(MARKDOWN_IMAGE_BYTES + 1)
        .read_to_end(&mut data)
        .ok()?;
    if data.len() as u64 > MARKDOWN_IMAGE_BYTES {
        return None;
    }
    let name = path.file_name().map(|n| n.to_string_lossy());
    let head = &data[..data.len().min(SNIFF_BYTES)];
    let svg = crate::image::is_svg(name.as_deref(), head);
    let opts = crate::image::ShowOptions {
        max_cols: Some(max_cols),
        max_rows: Some(MARKDOWN_IMAGE_ROWS),
        ..Default::default()
    };
    crate::image::draw_encoded(screen, data, svg, &opts).ok()
}

fn write_cat<W: Write>(
    input: &mut Input,
    out: &mut W,
    opts: &CatOptions,
    state: &mut CatState,
) -> io::Result<()> {
    if opts.is_passthrough() {
        cat::copy_raw(&mut input.reader, out)
    } else {
        cat::process(&mut input.reader, out, opts, state)
    }
}

/// Lazily created state shared by all files of one run.
struct Context<'a> {
    cli: &'a Cli,
    config: &'a Config,
    cat_options: CatOptions,
    colors: bool,
    depth: ColorDepth,
    utf8: bool,
    stdout_tty: bool,
    assets: Option<Assets>,
    caps: Option<Option<query::Capabilities>>,
    image_screen: Option<crate::image::Screen>,
    warned_theme: bool,
    /// Directory arguments being listed, and how many are done: headers for several.
    dir_args: usize,
    listed: usize,
    /// Print a rule with the file name before each file (several files, terminal).
    show_headers: bool,
    /// Files and listings printed so far, for the blank line between headed files.
    printed: usize,
    /// Diff or log colouring for the file being rendered.
    text_kind: Option<linestyle::Kind>,
}

impl<'a> Context<'a> {
    fn new(cli: &'a Cli, config: &'a Config, stdout_tty: bool) -> Self {
        let when = cli.color.unwrap_or(config.defaults.color);
        Self {
            cli,
            config,
            cat_options: CatOptions::from_cli(cli),
            colors: term::colors_enabled(when, stdout_tty),
            depth: term::color_depth(),
            utf8: term::utf8_locale(),
            stdout_tty,
            assets: None,
            caps: None,
            image_screen: None,
            warned_theme: false,
            dir_args: 0,
            listed: 0,
            show_headers: false,
            printed: 0,
            text_kind: None,
        }
    }

    fn capabilities(&mut self) -> Option<&query::Capabilities> {
        if self.caps.is_none() {
            self.caps = Some(term::capabilities(self.stdout_tty));
        }
        self.caps.as_ref().and_then(Option::as_ref)
    }

    /// How images are drawn in this run; queries the terminal on first use when the
    /// choice depends on it.
    pub(crate) fn image_screen(&mut self) -> crate::image::Screen {
        if let Some(screen) = &self.image_screen {
            return screen.clone();
        }
        let (protocol, protocol_source) = image_protocol(self.cli, self.config);
        let caps = if crate::image::needs_query(protocol, self.stdout_tty) {
            self.capabilities().cloned()
        } else {
            None
        };
        let screen = crate::image::Screen::detect(&crate::image::Env {
            protocol,
            protocol_source,
            caps: caps.as_ref(),
            stdout_tty: self.stdout_tty,
            utf8: self.utf8,
            colors: self.colors,
            depth: self.depth,
            multiplexer: term::multiplexer(),
            max_height: self.config.image.max_height,
        });
        self.image_screen = Some(screen.clone());
        screen
    }

    fn background_is_light(&mut self) -> bool {
        if let Some(light) = self.capabilities().and_then(|c| c.background_is_light()) {
            return light;
        }
        // COLORFGBG="fg;bg" (rxvt, konsole): bg 7 or 15 means a light background.
        term::hint("COLORFGBG")
            .and_then(|v| v.rsplit(';').next().and_then(|bg| bg.parse::<u8>().ok()))
            .is_some_and(|bg| bg == 7 || bg == 15)
    }

    /// The theme set by `--theme`, `SCAT_THEME`, a rule or `[defaults] theme`.
    fn fixed_theme(&self, settings: &FileSettings) -> Option<String> {
        self.cli.theme.clone().or_else(|| settings.theme.clone())
    }

    /// Whether markdown's inline code gets the light palette. With a fixed theme this
    /// follows the theme's own background, so no terminal query is needed.
    fn markdown_light(&mut self, settings: &FileSettings) -> bool {
        let Some(name) = self.fixed_theme(settings) else {
            return self.background_is_light();
        };
        let assets = self.assets.get_or_insert_with(Assets::load);
        assets
            .theme(&name)
            .and_then(|theme| theme.settings.background)
            .is_some_and(|c| {
                0.2126 * f64::from(c.r) + 0.7152 * f64::from(c.g) + 0.0722 * f64::from(c.b) > 128.0
            })
    }

    /// The highlighting theme. Asks the terminal for its background only when no theme
    /// is fixed and the colour depth allows more than the 16-colour `ansi` theme.
    fn theme_name(&mut self, settings: &FileSettings) -> String {
        if let Some(theme) = self.fixed_theme(settings) {
            return theme;
        }
        if self.depth == ColorDepth::Ansi16 {
            return "ansi".into();
        }
        if self.background_is_light() {
            self.config.defaults.theme_light.clone()
        } else {
            self.config.defaults.theme_dark.clone()
        }
    }
}

/// The image protocol setting and where it came from.
pub fn image_protocol(cli: &Cli, config: &Config) -> (crate::config::Protocol, &'static str) {
    match cli.protocol {
        Some(protocol) => (protocol, "--protocol or SCAT_PROTOCOL"),
        None => (config.image.protocol, "config"),
    }
}

fn render_image<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    source: &Source,
    head: &[u8],
) -> io::Result<()> {
    let screen = ctx.image_screen();
    let file_name = source.file_name();
    let options = crate::image::ViewOptions {
        name: &source.display_name(),
        file_size: match input.kind {
            InputKind::Regular { size } => Some(size),
            _ => None,
        },
        svg: crate::image::is_svg(file_name.as_deref(), head),
        animate: !ctx.cli.no_animate && ctx.config.image.animate,
        caption: !ctx.cli.plain,
    };
    crate::image::view(input, out, &screen, &options)
}

/// Mermaid, graphviz and PlantUML sources drawn as an image with a caption. Shown as
/// highlighted source when images cannot be drawn (ASCII output, or `-f` into a pipe
/// without `--protocol`), and after a short note when drawing fails.
fn render_diagram<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    settings: &FileSettings,
    file_name: Option<&str>,
    source: &Source,
) -> io::Result<()> {
    let mut data = Vec::new();
    (&mut input.reader)
        .take(diagram::MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut data)?;
    let fits = data.len() as u64 <= diagram::MAX_SOURCE_BYTES;
    let want_images = fits && (ctx.stdout_tty || ctx.cli.protocol.is_some());
    if want_images {
        let text = String::from_utf8_lossy(&data);
        let language = ctx.cli.language.as_deref().or(settings.language.as_deref());
        let kind = diagram::detect(file_name, language).or_else(|| diagram::sniff(&text));
        let screen = ctx.image_screen();
        let name = source.display_name();
        match kind {
            _ if screen.method == crate::image::Method::Ascii => {}
            Some(kind) => match draw_diagram(&screen, kind, &text, (None, None)) {
                Ok((drawn, engine)) => {
                    drawn.write(out)?;
                    if !ctx.cli.plain {
                        let caption = format!(
                            "{}  {} diagram ({})",
                            sanitize_str(&name),
                            kind.name(),
                            engine.name()
                        );
                        if ctx.colors {
                            writeln!(out, "\x1b[2m{caption}\x1b[22m")?;
                        } else {
                            writeln!(out, "{caption}")?;
                        }
                    }
                    return Ok(());
                }
                Err(message) => {
                    out.flush()?;
                    crate::report!(
                        "{name}: cannot draw {} diagram: {}; showing the source",
                        kind.name(),
                        sanitize_str(&message)
                    );
                }
            },
            None => {
                out.flush()?;
                crate::report!(
                    "{name}: not a mermaid, graphviz or PlantUML diagram; showing the source"
                );
            }
        }
    }
    let rest = std::mem::replace(&mut input.reader, BufReader::new(Box::new(io::empty())));
    input.reader = BufReader::new(Box::new(io::Cursor::new(data).chain(rest)));
    let head = sniff(input)?;
    render_text(input, out, ctx, settings, file_name, &head)
}

/// Renders a diagram to SVG and encodes it for `screen` within `(max_cols, max_rows)`.
/// Errors are short messages.
fn draw_diagram(
    screen: &crate::image::Screen,
    kind: diagram::Kind,
    text: &str,
    (max_cols, max_rows): (Option<u16>, Option<u16>),
) -> Result<(crate::image::Drawn, diagram::Engine), String> {
    let svg = diagram::render(kind, text).map_err(|failure| failure.to_string())?;
    let opts = crate::image::ShowOptions {
        max_cols,
        max_rows,
        ..Default::default()
    };
    let drawn = crate::image::draw_encoded(screen, svg.data, true, &opts)
        .map_err(|err| format!("{}: cannot draw the SVG: {err}", svg.engine.name()))?;
    Ok((drawn, svg.engine))
}

/// PDF view. Pages are drawn as images on a terminal; their text is shown with
/// `--text`, when only ASCII art is possible, and for `-f` into a pipe (unless
/// `--protocol` asks for images).
fn render_pdf<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    source: &Source,
) -> io::Result<()> {
    let want_images = !ctx.cli.text && (ctx.stdout_tty || ctx.cli.protocol.is_some());
    let screen = if want_images {
        Some(ctx.image_screen()).filter(|s| s.method != crate::image::Method::Ascii)
    } else {
        None
    };
    let name = source.display_name();
    let opts = crate::pdf::Options {
        name: &name,
        pages: ctx.cli.pages.as_ref().unwrap_or(&ctx.config.pdf.pages),
        max_pages: ctx.config.pdf.max_pages,
        screen,
        colors: ctx.colors,
        utf8: ctx.utf8,
        plain: ctx.cli.plain,
    };
    crate::pdf::render(out, source, input, &opts)
}

/// Archive listing. Needs the format found by sniffing; `--archive` style forcing of a
/// file that is not an archive reports an error instead of guessing.
fn render_archive<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &Context,
    source: &Source,
    format: Option<archive::Format>,
) -> io::Result<()> {
    let Some(format) = format else {
        return Err(io::Error::other("not a supported archive"));
    };
    let reader = std::mem::replace(&mut input.reader, BufReader::new(Box::new(io::empty())));
    let (path, size) = match (source, input.kind) {
        (Source::Path(path), InputKind::Regular { size }) => (Some(path.as_path()), Some(size)),
        _ => (None, None),
    };
    let name = source.display_name();
    let opts = archive::Options {
        max_entries: ctx.config.archive.max_entries,
        colors: ctx.colors,
        utf8: ctx.utf8,
        width: listing::terminal_width(),
        max_scan_bytes: ctx.config.defaults.max_decompressed_bytes,
    };
    let source = archive::Source {
        reader: Box::new(reader),
        path,
        size,
    };
    archive::render(out, format, &name, source, &opts)
}

/// Where a font specimen is drawn: on a terminal, or with `--protocol` into a pipe;
/// never as ASCII art. `None` means the info table alone.
fn font_screen(ctx: &mut Context) -> Option<crate::image::Screen> {
    if !ctx.stdout_tty && ctx.cli.protocol.is_none() {
        return None;
    }
    Some(ctx.image_screen()).filter(|s| s.method != crate::image::Method::Ascii)
}

/// Font view: info table per face, then the specimen when images can be drawn.
fn render_font<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    source: &Source,
) -> io::Result<()> {
    let name = source.display_name();
    let opts = crate::font::Options {
        name: &name,
        colors: ctx.colors,
        utf8: ctx.utf8,
        screen: font_screen(ctx),
    };
    crate::font::render(out, input, &opts)
}

/// SQLite database view; only regular files, SQLite needs to open the path itself.
fn render_sqlite<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &Context,
    source: &Source,
) -> io::Result<()> {
    let name = source.display_name();
    let opts = crate::sqlite::Options {
        name: &name,
        colors: ctx.colors,
        utf8: ctx.utf8,
        width: listing::terminal_width(),
        sample_rows: ctx.config.sqlite.sample_rows,
        max_tables: ctx.config.sqlite.max_tables,
    };
    // Built first so a failing database prints only the error, no half a view.
    let mut buf = Vec::new();
    crate::sqlite::render(&mut buf, source, input, &opts)?;
    out.write_all(&buf)
}

/// Subtitle view. The file is read up to a cap; a file that does not parse falls back
/// to the text view (or a hex dump for binary content) with a note on stderr.
fn render_subtitles<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    settings: &FileSettings,
    file_name: Option<&str>,
    head: &[u8],
) -> io::Result<()> {
    let language = ctx.cli.language.as_deref().or(settings.language.as_deref());
    let format = subtitles::wants(file_name, language, head)
        .or_else(|| file_name.and_then(|n| subtitles::wants(Some(n), None, head)))
        .unwrap_or(subtitles::Format::Srt);
    let mut data = Vec::new();
    (&mut input.reader)
        .take(subtitles::MAX_READ_BYTES + 1)
        .read_to_end(&mut data)?;
    let truncated = data.len() as u64 > subtitles::MAX_READ_BYTES;
    data.truncate(subtitles::MAX_READ_BYTES as usize);
    let name = file_name.unwrap_or("-").to_string();
    let cli = ctx.cli;
    let opts = subtitles::Options {
        name: &name,
        colors: ctx.colors,
        utf8: ctx.utf8,
        max_cues: ctx.config.subtitles.max_cues,
        line_numbers: !cli.plain && (cli.number || cli.number_nonblank || settings.line_numbers),
    };
    match subtitles::render(out, format, &data, truncated, file_name, &opts) {
        Ok(()) => return Ok(()),
        Err(subtitles::Failure::Io(err)) => return Err(err),
        Err(subtitles::Failure::Invalid(reason)) => {
            crate::report!("{name}: not a valid subtitle file ({reason}); showing it as text");
        }
    }
    let binary = detect::detect(&data[..data.len().min(SNIFF_BYTES)]).kind == Kind::Binary
        && !data.starts_with(b"\xFF\xFE")
        && !data.starts_with(b"\xFE\xFF");
    let rest = if truncated {
        std::mem::replace(&mut input.reader, BufReader::new(Box::new(io::empty())))
    } else {
        BufReader::new(Box::new(io::empty()) as Box<dyn Read>)
    };
    input.reader = BufReader::new(Box::new(io::Cursor::new(data).chain(rest)));
    if binary {
        render_hex(input, out, ctx)
    } else {
        let head = sniff(input)?;
        render_text(input, out, ctx, settings, file_name, &head)
    }
}

fn render_text<W: Write>(
    input: &mut Input,
    out: &mut W,
    ctx: &mut Context,
    settings: &FileSettings,
    file_name: Option<&str>,
    head: &[u8],
) -> io::Result<()> {
    let cli = ctx.cli;
    let line_numbers = !cli.plain && (cli.number || cli.number_nonblank || settings.line_numbers);
    let squeeze = cli.squeeze_blank;
    let tab_width = settings.tab_width;
    let small_enough = match input.kind {
        InputKind::Regular { size } => size <= ctx.config.defaults.max_highlight_bytes,
        _ => true,
    };
    let sgr_mode = match (ctx.config.defaults.ansi, ctx.colors) {
        (AnsiMode::Escape, _) => SgrMode::Escape,
        (AnsiMode::KeepSgr, true) => SgrMode::Keep,
        (AnsiMode::KeepSgr, false) => SgrMode::Strip,
    };

    let mut styler = ctx
        .text_kind
        .filter(|_| ctx.colors)
        .map(linestyle::Styler::new);
    // An empty file has nothing to highlight.
    let want_highlight = ctx.colors
        && settings.renderer != Renderer::Text
        && small_enough
        && ctx.text_kind.is_none()
        && !head.is_empty();
    if want_highlight && ctx.assets.is_none() {
        ctx.assets = Some(Assets::load());
    }
    let first_line = String::from_utf8_lossy(head.split(|&b| b == b'\n').next().unwrap_or(&[]));
    let first_line = format!("{first_line}\n");
    let language = cli.language.as_deref().or(settings.language.as_deref());
    // The theme (and with it the terminal query for the background) is only needed
    // once a syntax is found: plain text is never highlighted.
    let has_syntax = want_highlight
        && ctx
            .assets
            .as_ref()
            .is_some_and(|a| a.find_syntax(language, file_name, &first_line).is_some());
    if want_highlight
        && !has_syntax
        && let Some(language) = language
    {
        crate::report!("unknown language '{language}'; see --list-languages");
    }
    let theme_name = has_syntax.then(|| ctx.theme_name(settings));
    let mut highlighter = None;
    if let (Some(assets), Some(theme_name)) = (ctx.assets.as_ref(), theme_name) {
        let syntax = assets.find_syntax(language, file_name, &first_line);
        let theme = assets.theme(&theme_name).or_else(|| {
            if !ctx.warned_theme {
                crate::report!("unknown theme '{theme_name}'; see --list-themes");
                ctx.warned_theme = true;
            }
            assets.theme(&ctx.config.defaults.theme_dark)
        });
        if let (Some(syntax), Some(theme)) = (syntax, theme) {
            highlighter = Some(HighlightLines::new(syntax, theme));
        }
    }
    let syntaxes = ctx.assets.as_ref().map(|a| &a.syntaxes);

    let separator = if ctx.utf8 { "│" } else { "|" };
    let (dim, undim) = if ctx.colors {
        ("\x1b[2m", "\x1b[22m")
    } else {
        ("", "")
    };

    let mut line = Vec::with_capacity(8 * 1024);
    let mut number = 0u64;
    let mut blank_run = 0u32;
    let mut at_line_start = true;
    let mut passed_sgr = false;
    let mut encoded = Vec::with_capacity(8 * 1024);

    loop {
        if input.reader.buffer().is_empty() {
            out.flush()?;
        }
        let Some(terminated) = read_line_capped(&mut input.reader, &mut line, MAX_RENDER_LINE)?
        else {
            break;
        };
        let body_len = if line.ends_with(b"\r\n") {
            line.len() - 2
        } else if terminated {
            line.len() - 1
        } else {
            line.len()
        };

        if at_line_start {
            let blank = terminated && body_len == 0;
            if blank {
                if squeeze && blank_run > 0 {
                    continue;
                }
                blank_run += 1;
            } else {
                blank_run = 0;
            }
            if line_numbers {
                if blank && cli.number_nonblank {
                    write!(out, "{dim}     {separator}{undim} ")?;
                } else {
                    number += 1;
                    write!(out, "{dim}{number:>4} {separator}{undim} ")?;
                }
            }
        }
        if !terminated {
            // A partial line breaks the highlighter's line-by-line state.
            highlighter = None;
        }

        encoded.clear();
        let mut column = 0usize;
        let text = String::from_utf8_lossy(&line);
        let mut highlighted = false;
        if let (Some(hl), Some(syntaxes)) = (highlighter.as_mut(), syntaxes) {
            match hl.highlight_line(&text, syntaxes) {
                Ok(regions) => {
                    for (style, piece) in regions {
                        let piece = piece.trim_end_matches(['\n', '\r']);
                        if piece.is_empty() {
                            continue;
                        }
                        encoded
                            .extend_from_slice(highlight::style_sgr(style, ctx.depth).as_bytes());
                        sanitize(
                            &mut encoded,
                            piece.as_bytes(),
                            SgrMode::Escape,
                            tab_width,
                            &mut column,
                        );
                    }
                    encoded.extend_from_slice(RESET);
                    highlighted = true;
                }
                Err(_) => highlighter = None,
            }
        }
        if !highlighted
            && at_line_start
            && !text.contains('\x1b')
            && let Some(styler) = styler.as_mut()
        {
            let body = text.strip_suffix('\n').unwrap_or(&text);
            let body = body.strip_suffix('\r').unwrap_or(body);
            for (range, sgr) in styler.spans(body) {
                encoded.extend_from_slice(sgr.as_bytes());
                sanitize(
                    &mut encoded,
                    &body.as_bytes()[range],
                    SgrMode::Escape,
                    tab_width,
                    &mut column,
                );
                if !sgr.is_empty() {
                    encoded.extend_from_slice(RESET);
                }
            }
            highlighted = true;
        }
        if !highlighted {
            let body = text.strip_suffix('\n').unwrap_or(&text);
            let body = body.strip_suffix('\r').unwrap_or(body);
            passed_sgr |= sanitize(
                &mut encoded,
                body.as_bytes(),
                sgr_mode,
                tab_width,
                &mut column,
            );
        }
        if terminated {
            encoded.push(b'\n');
        }
        out.write_all(&encoded)?;
        at_line_start = terminated;
    }
    if passed_sgr {
        out.write_all(RESET)?;
    }
    Ok(())
}

/// Sanitizes a short string from a file (a tag, a title) for display: control
/// characters become ^X, escape sequences cannot pass, tabs become spaces.
pub fn sanitize_str(text: &str) -> String {
    let mut out = Vec::with_capacity(text.len());
    let mut column = 0;
    sanitize(&mut out, text.as_bytes(), SgrMode::Escape, 1, &mut column);
    String::from_utf8_lossy(&out).into_owned()
}

/// Sanitizes one line of a file for display, expanding tabs to `tab_width` columns.
pub fn sanitize_line(text: &str, tab_width: usize) -> String {
    let mut out = Vec::with_capacity(text.len());
    let mut column = 0;
    sanitize(
        &mut out,
        text.as_bytes(),
        SgrMode::Escape,
        tab_width,
        &mut column,
    );
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SgrMode {
    Keep,
    Strip,
    Escape,
}

/// Makes text safe to print on a terminal: expands tabs, shows control characters in
/// ^X notation and keeps, strips or escapes SGR colour sequences. `text` must be valid
/// UTF-8. Returns true if any SGR sequence was passed through.
fn sanitize(
    out: &mut Vec<u8>,
    text: &[u8],
    mode: SgrMode,
    tab_width: usize,
    column: &mut usize,
) -> bool {
    let mut passed_sgr = false;
    let mut i = 0;
    while i < text.len() {
        let byte = text[i];
        if byte == 0x1b && mode != SgrMode::Escape && text.get(i + 1) == Some(&b'[') {
            let mut end = i + 2;
            while end < text.len()
                && (text[end].is_ascii_digit() || text[end] == b';' || text[end] == b':')
            {
                end += 1;
            }
            if text.get(end) == Some(&b'm') {
                if mode == SgrMode::Keep {
                    out.extend_from_slice(&text[i..=end]);
                    passed_sgr = true;
                }
                i = end + 1;
                continue;
            }
        }
        match byte {
            b'\t' if tab_width > 0 => {
                let spaces = tab_width - (*column % tab_width);
                out.extend(std::iter::repeat_n(b' ', spaces));
                *column += spaces;
            }
            b'\t' => {
                out.push(b'\t');
                *column += 1;
            }
            0..=0x1f | 0x7f => {
                cat::push_caret_notation(out, byte);
                *column += 2;
            }
            // C1 control characters (U+0080..U+009F) encoded as UTF-8.
            0xc2 if text.get(i + 1).is_some_and(|b| (0x80..=0x9f).contains(b)) => {
                out.extend_from_slice("\u{fffd}".as_bytes());
                *column += 1;
                i += 1;
            }
            // Bidi controls reorder what is shown (Trojan Source); show them instead.
            0xd8 | 0xe2 if bidi_control(&text[i..]).is_some() => {
                let (code, len) = bidi_control(&text[i..]).unwrap_or_default();
                let shown = format!("<U+{code:04X}>");
                out.extend_from_slice(shown.as_bytes());
                *column += shown.len();
                i += len - 1;
            }
            _ => {
                out.push(byte);
                if byte & 0xc0 != 0x80 {
                    *column += 1;
                }
            }
        }
        i += 1;
    }
    passed_sgr
}

/// The code point and UTF-8 length of a bidirectional formatting control at the start
/// of `bytes`: U+061C, U+200E, U+200F, U+202A..U+202E and U+2066..U+2069.
fn bidi_control(bytes: &[u8]) -> Option<(u32, usize)> {
    match *bytes {
        [0xd8, 0x9c, ..] => Some((0x061c, 2)),
        [0xe2, 0x80, b @ (0x8e | 0x8f | 0xaa..=0xae), ..] => {
            Some((0x2000 + u32::from(b - 0x80), 3))
        }
        [0xe2, 0x81, b @ 0xa6..=0xa9, ..] => Some((0x2040 + u32::from(b - 0x80), 3)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(text: &str, mode: SgrMode) -> String {
        let mut out = Vec::new();
        let mut column = 0;
        sanitize(&mut out, text.as_bytes(), mode, 4, &mut column);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn escapes_dangerous_sequences() {
        // Window title and clipboard (OSC 52) sequences must never reach the terminal.
        assert_eq!(clean("a\x1b]0;pwned\x07b", SgrMode::Keep), "a^[]0;pwned^Gb");
        assert_eq!(clean("\x1b]52;c;aGk=\x07", SgrMode::Keep), "^[]52;c;aGk=^G");
        assert_eq!(clean("x\u{9b}31m", SgrMode::Keep), "x\u{fffd}31m");
    }

    #[test]
    fn shows_bidi_controls() {
        // Trojan Source: an RLO/LRI must not reorder the line silently.
        assert_eq!(
            clean("a\u{202e}b\u{2066}c\u{200f}d\u{61c}e", SgrMode::Keep),
            "a<U+202E>b<U+2066>c<U+200F>d<U+061C>e"
        );
        // Ordinary right-to-left text is left alone.
        assert_eq!(clean("שלום مرحبا", SgrMode::Keep), "שלום مرحبا");
        assert_eq!(clean("\u{2010}\u{2070}", SgrMode::Keep), "\u{2010}\u{2070}");
    }

    #[test]
    fn handles_sgr_by_mode() {
        let red = "\x1b[31mred\x1b[0m";
        assert_eq!(clean(red, SgrMode::Keep), red);
        assert_eq!(clean(red, SgrMode::Strip), "red");
        assert_eq!(clean(red, SgrMode::Escape), "^[[31mred^[[0m");
    }

    #[test]
    fn sanitizes_short_strings() {
        assert_eq!(sanitize_str("Rose\x1b]0;x\x07\tmix"), "Rose^[]0;x^G mix");
    }

    #[test]
    fn expands_tabs_to_tab_stops() {
        assert_eq!(clean("a\tbc\td", SgrMode::Keep), "a   bc  d");
        assert_eq!(clean("é\tx", SgrMode::Keep), "é   x");
    }
}
