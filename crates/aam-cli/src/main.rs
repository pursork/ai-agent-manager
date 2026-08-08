//! `aam` CLI entry point (`docs/03-credential-account-module.md` §3.6).

mod cli;
mod commands;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    if let Err(e) = commands::run(cli.command) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
