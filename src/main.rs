mod audio;
mod cat;
mod cli;
mod config;
mod decompress;
mod detect;
mod diagnose;
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
mod office;
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

    let cli = Cli::parse();
    std::process::exit(run(&cli));
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
