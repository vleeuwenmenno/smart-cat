mod archive;
mod audio;
mod binary;
mod cat;
mod cli;
mod config;
mod decompress;
mod detect;
mod diagnose;
mod fastcopy;
mod generate;
mod header;
mod hexdump;
mod highlight;
mod image;
mod input;
mod json;
mod kv;
mod linestyle;
mod listing;
mod markdown;
mod pager;
mod pdf;
mod play;
mod render;
mod table;
mod term;
mod update;
mod util;
mod video;
mod waveform;

use clap::Parser;

use crate::cli::Cli;

fn main() {
    // Behave like cat when the reader goes away (`scat file | head`): die quietly on SIGPIPE
    // instead of reporting a broken pipe error.
    // SAFETY: setting a signal disposition to SIG_DFL at startup, before any threads exist.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    if let Some(code) = pdf::internal_main() {
        std::process::exit(code);
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => usage_error(err),
    };
    std::process::exit(run(&cli));
}

/// Reports a command line error like GNU cat: `scat: invalid option -- 'z'`, a hint
/// and exit status 1 (clap's own default is 2, which scripts checking for cat's
/// status would not expect). `--help` and `--version` print and exit 0 as usual.
fn usage_error(err: clap::Error) -> ! {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    if matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        err.exit();
    }
    let invalid = match err.get(ContextKind::InvalidArg) {
        Some(ContextValue::String(arg)) => Some(arg.clone()),
        _ => None,
    };
    let message = match (err.kind(), invalid) {
        (ErrorKind::UnknownArgument, Some(arg)) if arg.starts_with("--") => {
            format!("unrecognized option '{arg}'")
        }
        (ErrorKind::UnknownArgument, Some(arg)) if arg.starts_with('-') => {
            format!("invalid option -- '{}'", &arg[1..])
        }
        _ => {
            let text = err.render().to_string();
            let first = text.lines().next().unwrap_or_default();
            first.strip_prefix("error: ").unwrap_or(first).to_string()
        }
    };
    eprintln!("scat: {}", render::sanitize_str(&message));
    eprintln!("Try 'scat --help' for more information.");
    std::process::exit(1);
}

fn run(cli: &Cli) -> i32 {
    if let Some(shell) = cli.completions {
        return generate::completions(shell);
    }
    if cli.man {
        return generate::man();
    }
    if cli.install_completions {
        return generate::install(generate::Mode::Install);
    }
    if cli.uninstall_completions {
        return generate::install(generate::Mode::Uninstall);
    }
    if cli.refresh_completions {
        return generate::install(generate::Mode::Refresh);
    }
    let loaded = config::load(cli.config.as_deref(), cli.no_config);
    for warning in &loaded.warnings {
        eprintln!("scat: {warning}");
    }
    let config = &loaded.config;

    if cli.update || cli.check_update {
        return update::run(cli.check_update, cli.pre, config);
    }
    if cli.init_config {
        return config::init(cli.config.as_deref());
    }
    if cli.print_config {
        return config::print(config);
    }
    if cli.list_themes {
        return highlight::list_themes();
    }
    if cli.list_languages {
        return highlight::list_languages();
    }
    if cli.diagnose {
        return diagnose::run(cli, &loaded);
    }
    if cli.play {
        return play::run(cli);
    }
    render::run(cli, config)
}
