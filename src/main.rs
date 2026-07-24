mod add;
mod cli;
mod clone;
mod convert;
mod list;
mod repo;
mod rm;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Commands::Convert => {
            convert::run()?;
        }
        cli::Commands::Clone { url, dir, git_flags } => {
            clone::run(&url, dir.as_deref(), &git_flags)?;
        }
        cli::Commands::Add { branch, print_path } => {
            add::run(&branch, print_path)?;
        }
        cli::Commands::Rm { branch, force } => {
            rm::run(&branch, force)?;
        }
        cli::Commands::List { verbose, color } => {
            list::run(verbose, &color)?;
        }
    }
    Ok(())
}
