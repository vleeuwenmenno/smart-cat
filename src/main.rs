mod audio;
mod cat;
mod cli;
mod config;
mod detect;
mod diagnose;
mod highlight;
mod input;
mod kv;
mod listing;
mod play;
mod render;
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

    let cli = Cli::parse();
    std::process::exit(run(&cli));
}

fn run(cli: &Cli) -> i32 {
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
