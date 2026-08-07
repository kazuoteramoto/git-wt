mod add;
mod cli;
mod clone;
mod convert;
mod list;
mod repo;
mod rm;
mod ssh;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    match cli.command {
        cli::Commands::Convert => {
            convert::run()?;
        }
        cli::Commands::Clone {
            url,
            dir,
            branch,
            depth,
            origin,
        } => {
            clone::run(
                &url,
                dir.as_deref(),
                branch.as_deref(),
                depth,
                origin.as_deref().unwrap_or("origin"),
            )?;
        }
        cli::Commands::Add { branch, print_path } => {
            add::run(&branch, print_path)?;
        }
        cli::Commands::Rm { branch, force } => {
            rm::run(&branch, force)?;
        }
        cli::Commands::List { verbose, color } => {
            list::run(verbose, color)?;
        }
    }
    Ok(())
}
