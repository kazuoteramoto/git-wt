use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "wt", version, about = "Manage git worktrees with a branch-as-a-folder layout")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Clone a repository into a worktree-managed layout
    Clone {
        /// Remote URL to clone from
        url: String,
        /// Target directory (defaults to repo name derived from URL)
        dir: Option<String>,
        /// Extra flags forwarded to git clone (everything after --)
        #[arg(last = true)]
        git_flags: Vec<String>,
    },
    /// Add a new worktree for the given branch
    Add {
        /// Branch name to create and check out
        branch: String,
        /// Print the absolute path of the new worktree to stdout
        #[arg(short = 'p', long = "print-path")]
        print_path: bool,
    },
    /// Remove a worktree and delete its branch
    Rm {
        /// Branch name to remove
        branch: String,
        /// Force removal even if branch has unmerged commits
        #[arg(short = 'f', long = "force")]
        force: bool,
    },
    /// List worktrees (format matches git branch -v)
    List {
        /// Show path and status columns
        #[arg(short = 'v', long = "verbose")]
        verbose: bool,
        /// When to use colors: always, never, or auto
        #[arg(long = "color", default_value = "auto")]
        color: String,
    },
}
