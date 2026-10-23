//! `scat --diagnose`: what scat detected about the terminal, config and files.

use std::io::{BufRead, IsTerminal};

use crate::cli::Cli;
use crate::config::{Loaded, Renderer};
use crate::detect::{self, Kind, SNIFF_BYTES};
use crate::highlight::Assets;
use crate::input::{self, InputKind, Source};
use crate::term::{self, query};
use crate::util::{human_size, io_message, which};

const TOOLS: &[(&str, &str)] = &[
    ("pdftoppm", "PDF pages"),
    ("pdftotext", "PDF text"),
    ("ffmpeg", "video frames"),
    ("ffprobe", "media metadata"),
    ("mpv", "playback"),
    ("less", "paging"),
];

pub fn run(cli: &Cli, loaded: &Loaded) -> i32 {
    let config = &loaded.config;
    println!("scat {}", env!("CARGO_PKG_VERSION"));

    println!("\nconfig");
    match (&loaded.path, &loaded.default_path) {
        (Some(path), _) => println!("  file: {} (loaded)", path.display()),
        (None, Some(path)) if cli.no_config => {
            println!("  file: {} (ignored: --no-config)", path.display())
        }
        (None, Some(path)) => println!("  file: {} (not found, using defaults)", path.display()),
        (None, None) => println!("  file: none (HOME is not set)"),
    }
    for warning in &loaded.warnings {
        println!("  warning: {warning}");
    }
    println!("  rules: {}", config.rules.len());

    let stdout_tty = std::io::stdout().is_terminal();
    let when = cli.color.unwrap_or(config.defaults.color);
    println!("\noutput");
    println!(
        "  stdout: {}",
        if stdout_tty {
            "terminal (rendering on)"
        } else {
            "not a terminal (cat-compatible output unless --force-render)"
        }
    );
    println!(
        "  colour: {} ({:?})",
        if term::colors_enabled(when, stdout_tty) {
            "on"
        } else {
            "off"
        },
        when
    );
    println!("  colour depth: {}", term::color_depth().name());
    println!("  UTF-8 locale: {}", yes_no(term::utf8_locale()));
    println!("  multiplexer: {}", term::multiplexer().unwrap_or("none"));

    println!("\nenvironment");
    for name in term::HINT_VARS {
        if let Some(value) = term::hint(name) {
            println!("  {name}={value}");
        }
    }

    println!("\nterminal query (/dev/tty)");
    let caps = if term::is_basic_terminal() {
        println!(
            "  skipped: TERM={} does not answer queries",
            term::term_name().unwrap_or_default()
        );
        None
    } else {
        match query::query(query::DEFAULT_TIMEOUT) {
            Ok(caps) => Some(caps),
            Err(err) => {
                println!("  failed: {}", io_message(&err));
                None
            }
        }
    };
    if let Some(caps) = &caps {
        println!(
            "  answered: {} in {} ms",
            yes_no(caps.answered),
            caps.elapsed.as_millis()
        );
        if let Some(version) = &caps.version {
            println!("  terminal: {version}");
        }
        println!("  kitty graphics: {}", yes_no(caps.kitty_graphics));
        println!("  sixel: {}", yes_no(caps.sixel));
        if let Some((w, h)) = caps.cell_pixels {
            println!("  cell size: {w}x{h} px");
        }
        if let Some((w, h)) = caps.window_pixels {
            println!("  text area: {w}x{h} px");
        }
        if let Some((r, g, b)) = caps.background {
            let shade = if caps.background_is_light() == Some(true) {
                "light"
            } else {
                "dark"
            };
            println!("  background: #{r:02x}{g:02x}{b:02x} ({shade})");
        }
    }

    let (protocol, protocol_source) = crate::render::image_protocol(cli, config);
    let detect = |tty: bool| {
        crate::image::Screen::detect(&crate::image::Env {
            protocol,
            protocol_source,
            caps: caps.as_ref().filter(|c| c.answered),
            stdout_tty: tty,
            utf8: term::utf8_locale(),
            colors: term::colors_enabled(when, tty),
            depth: term::color_depth(),
            multiplexer: term::multiplexer(),
            max_height: config.image.max_height,
        })
    };
    let screen = detect(stdout_tty);
    println!("\nimages");
    println!("  protocol setting: {protocol} ({protocol_source})");
    if stdout_tty {
        println!("  output: {} ({})", screen.method.name(), screen.reason);
    } else {
        let on_tty = detect(true);
        println!(
            "  on the terminal: {} ({})",
            on_tty.method.name(),
            on_tty.reason
        );
        println!(
            "  with -f into this pipe: {} ({})",
            screen.method.name(),
            screen.reason
        );
    }
    println!(
        "  size limit: {} columns x {} rows (terminal {}x{})",
        screen.cols, screen.max_rows, screen.cols, screen.rows
    );
    println!(
        "  cell size: {}x{} px ({})",
        screen.cell.0,
        screen.cell.1,
        if screen.cell_known {
            "reported by the terminal"
        } else {
            "assumed"
        }
    );
    println!(
        "  animation: {}",
        if cli.no_animate || !config.image.animate {
            "off (first frame only)"
        } else {
            "play once"
        }
    );

    println!("\ntools");
    for (tool, purpose) in TOOLS {
        match which(tool) {
            Some(path) => println!("  {tool}: {} ({purpose})", path.display()),
            None => println!("  {tool}: not found ({purpose})"),
        }
    }

    if !cli.files.is_empty() {
        println!("\nfiles");
        let assets = Assets::load();
        for source in Source::from_args(&cli.files) {
            describe(&source, cli, loaded, &assets);
        }
    }
    0
}

fn describe(source: &Source, cli: &Cli, loaded: &Loaded, assets: &Assets) {
    let name = source.display_name();
    if matches!(source, Source::Stdin) {
        println!("  {name}: standard input (not inspected)");
        return;
    }
    let mut input = match input::open(source) {
        Ok(input) => input,
        Err(err) => {
            println!("  {name}: {}", io_message(&err));
            return;
        }
    };
    if input.kind == InputKind::Device {
        println!("  {name}: device, printed as-is");
        return;
    }
    let head = match input.reader.fill_buf() {
        Ok(head) => head[..head.len().min(SNIFF_BYTES)].to_vec(),
        Err(err) => {
            println!("  {name}: {}", io_message(&err));
            return;
        }
    };
    let detected = detect::detect(&head);
    let file_name = source.file_name();
    let settings = loaded
        .config
        .settings_for(file_name.as_deref(), &detected.mime);
    let size = match input.kind {
        InputKind::Regular { size } => human_size(size),
        _ => "stream".into(),
    };
    println!("  {name}");
    println!(
        "    type: {} ({}), {size}",
        detected.mime,
        if detected.kind == Kind::Text {
            "text"
        } else {
            "binary"
        }
    );
    let is_image = crate::image::wants(&detected.mime, file_name.as_deref(), &head);
    let is_pdf = crate::pdf::wants(&detected.mime, &head);
    let renderer = match (settings.renderer, &detected.kind) {
        (Renderer::Auto, _) if is_pdf => "pdf (auto)".to_string(),
        (Renderer::Auto, _) if is_image => "image (auto)".to_string(),
        (Renderer::Auto, _) if detected.mime.starts_with("video/") => "video (auto)".to_string(),
        (Renderer::Auto, _) if crate::render::is_audio(&detected, file_name.as_deref()) => {
            "audio (auto)".to_string()
        }
        (Renderer::Auto, Kind::Binary) if crate::binary::wants(&head) => {
            "binary (auto)".to_string()
        }
        (Renderer::Auto, Kind::Binary) => "hex (auto)".to_string(),
        (Renderer::Auto, Kind::Text)
            if crate::markdown::is_markdown(
                file_name.as_deref(),
                cli.language.as_deref().or(settings.language.as_deref()),
            ) =>
        {
            "markdown (auto)".to_string()
        }
        (Renderer::Auto, Kind::Text)
            if crate::table::delimiter_for(
                file_name.as_deref(),
                cli.language.as_deref().or(settings.language.as_deref()),
            )
            .is_some() =>
        {
            "table (auto)".to_string()
        }
        (Renderer::Auto, Kind::Text)
            if crate::render::text_kind(
                Renderer::Auto,
                cli.language.as_deref().or(settings.language.as_deref()),
                file_name.as_deref(),
                &head,
            )
            .is_some() =>
        {
            format!(
                "{} (auto)",
                if crate::render::text_kind(
                    Renderer::Auto,
                    cli.language.as_deref().or(settings.language.as_deref()),
                    file_name.as_deref(),
                    &head,
                ) == Some(crate::linestyle::Kind::Diff)
                {
                    "diff"
                } else {
                    "log"
                }
            )
        }
        (Renderer::Auto, Kind::Text) => "code (auto)".to_string(),
        (other, _) => other.to_string(),
    };
    println!("    renderer: {renderer}");
    if renderer.starts_with("pdf") {
        let tool = |name| {
            if crate::util::which(name).is_some() {
                "yes"
            } else {
                "no"
            }
        };
        println!(
            "    pdf: hayro (built in); pdftoppm: {}, pdftotext: {}",
            tool("pdftoppm"),
            tool("pdftotext")
        );
    }
    if detected.kind == Kind::Text
        && !matches!(
            settings.renderer,
            Renderer::Raw
                | Renderer::Audio
                | Renderer::Image
                | Renderer::Table
                | Renderer::Hex
                | Renderer::Binary
                | Renderer::Pdf
        )
        && !(settings.renderer == Renderer::Auto && (is_image || is_pdf))
    {
        let first_line = String::from_utf8_lossy(head.split(|&b| b == b'\n').next().unwrap_or(&[]));
        let language = cli.language.as_deref().or(settings.language.as_deref());
        let syntax = assets.find_syntax(language, file_name.as_deref(), &format!("{first_line}\n"));
        println!(
            "    syntax: {}",
            syntax.map_or("none (plain text)", |s| s.name.as_str())
        );
    }
    if renderer.starts_with("markdown") {
        println!("    line numbers: no (rendered markdown)");
    } else {
        println!(
            "    line numbers: {}",
            yes_no(cli.number || cli.number_nonblank || settings.line_numbers)
        );
    }
    if let Some(theme) = cli.theme.as_ref().or(settings.theme.as_ref()) {
        println!("    theme: {theme}");
    }
    if settings.matched_rules.is_empty() {
        println!("    rules: none matched");
    } else {
        let list: Vec<String> = settings
            .matched_rules
            .iter()
            .map(|n| format!("#{n}"))
            .collect();
        println!("    rules: {}", list.join(", "));
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}
