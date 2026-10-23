//! Per-file dispatch: cat-compatible output or rendering.

use std::io::{self, BufRead, BufWriter, IsTerminal, Write};

use syntect::easy::HighlightLines;

use crate::cat::{self, CatOptions, CatState};
use crate::cli::Cli;
use crate::config::{AnsiMode, Config, FileSettings, Renderer};
use crate::detect::{self, Kind, SNIFF_BYTES};
use crate::highlight::{self, Assets};
use crate::input::{self, Input, InputKind, Source};
use crate::term::{self, ColorDepth, query};
use crate::util::{TrackedWriter, human_size, io_message, read_line_capped};

/// Lines longer than this are shown unhighlighted from that point on.
const MAX_RENDER_LINE: usize = 1024 * 1024;
const RESET: &[u8] = b"\x1b[0m";

pub fn run(cli: &Cli, config: &Config) -> i32 {
    let stdout = io::stdout();
    let stdout_tty = stdout.is_terminal();
    let render = !cli.raw && (stdout_tty || cli.force_render) && !cli.wants_cat_visualisation();
    let out_identity = input::stdout_identity();

    let mut ctx = Context::new(cli, config, stdout_tty);
    let mut out = TrackedWriter::new(BufWriter::with_capacity(64 * 1024, stdout.lock()));
    let mut cat_state = CatState::default();
    let mut status = 0;

    for source in Source::from_args(&cli.files) {
        let result = process(
            &source,
            render,
            out_identity,
            &mut ctx,
            &mut out,
            &mut cat_state,
        );
        if let Err(err) = result {
            if out.failed {
                eprintln!("scat: write error: {}", io_message(&err));
                return 1;
            }
            let _ = out.flush();
            eprintln!("scat: {}: {}", source.display_name(), io_message(&err));
            status = 1;
        }
    }
    if let Err(err) = out.flush() {
        eprintln!("scat: write error: {}", io_message(&err));
        return 1;
    }
    status
}

fn process<W: Write>(
    source: &Source,
    render: bool,
    out_identity: Option<(u64, u64)>,
    ctx: &mut Context,
    out: &mut TrackedWriter<W>,
    cat_state: &mut CatState,
) -> io::Result<()> {
    let mut input = input::open(source)?;
    if let InputKind::Regular { size } = input.kind
        && size > 0
        && out_identity.is_some()
        && out_identity == input.identity
    {
        return Err(io::Error::other("input file is output file"));
    }

    if !render || input.kind == InputKind::Device {
        return write_cat(&mut input, out, &ctx.cat_options, cat_state);
    }

    let head = input.reader.fill_buf()?;
    let head = head[..head.len().min(SNIFF_BYTES)].to_vec();
    let detected = detect::detect(&head);
    let file_name = source.file_name();
    let settings = ctx
        .config
        .settings_for(file_name.as_deref(), &detected.mime);

    match settings.renderer {
        Renderer::Raw => write_cat(&mut input, out, &ctx.cat_options, cat_state),
        Renderer::Image => render_image(&mut input, out, ctx, source, &head),
        Renderer::Auto if crate::image::wants(&detected.mime, file_name.as_deref(), &head) => {
            render_image(&mut input, out, ctx, source, &head)
        }
        Renderer::Auto if detected.kind == Kind::Binary => {
            out.flush()?;
            let size = match input.kind {
                InputKind::Regular { size } => format!(", {}", human_size(size)),
                _ => String::new(),
            };
            eprintln!(
                "scat: {}: binary file ({}{size}) not shown; use --raw to print it",
                source.display_name(),
                detected.mime
            );
            Ok(())
        }
        _ => render_text(&mut input, out, ctx, &settings, file_name.as_deref(), &head),
    }
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
        }
    }

    fn capabilities(&mut self) -> Option<&query::Capabilities> {
        if self.caps.is_none() {
            let caps = if self.stdout_tty && !term::is_basic_terminal() {
                query::query(query::DEFAULT_TIMEOUT)
                    .ok()
                    .filter(|c| c.answered)
            } else {
                None
            };
            self.caps = Some(caps);
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

    fn theme_name(&mut self, settings: &FileSettings) -> String {
        if let Some(theme) = self.cli.theme.clone().or_else(|| settings.theme.clone()) {
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

    let want_highlight = ctx.colors && settings.renderer != Renderer::Text && small_enough;
    let theme_name = if want_highlight {
        Some(ctx.theme_name(settings))
    } else {
        None
    };
    if want_highlight && ctx.assets.is_none() {
        ctx.assets = Some(Assets::load());
    }
    let mut highlighter = None;
    if let (Some(assets), Some(theme_name)) = (ctx.assets.as_ref(), theme_name) {
        let first_line = String::from_utf8_lossy(head.split(|&b| b == b'\n').next().unwrap_or(&[]));
        let language = cli.language.as_deref().or(settings.language.as_deref());
        let syntax = assets.find_syntax(language, file_name, &format!("{first_line}\n"));
        if syntax.is_none()
            && let Some(language) = language
        {
            eprintln!("scat: unknown language '{language}'; see --list-languages");
        }
        let theme = assets.theme(&theme_name).or_else(|| {
            if !ctx.warned_theme {
                eprintln!("scat: unknown theme '{theme_name}'; see --list-themes");
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
