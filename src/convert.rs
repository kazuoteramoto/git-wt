use anyhow::{bail, Context, Result};
use git2::Repository;
use std::path::PathBuf;

use crate::repo;

/// Convert the current directory (must be a plain git repo) to the
/// branch-as-a-folder layout. The working tree moves into a subdirectory
/// named after the current branch.
pub fn run() -> Result<()> {
    let cwd = std::env::current_dir().context("failed to get current directory")?;

    // Guard: .git must be a directory (not a gitlink file from a worktree)
    let dot_git = cwd.join(".git");
    if !dot_git.is_dir() {
        if dot_git.is_file() {
            bail!("this is a linked worktree — convert only works on a plain git repository");
        }
        bail!("not a git repository (no .git directory)");
    }

    // Open the repo
    let repo = Repository::open(&cwd).context("failed to open git repository")?;

    // Guard: not bare
    if repo.is_bare() {
        bail!("bare repositories cannot be converted");
    }

    // Guard: core.worktree not already set (already converted)
    // Canonicalize both paths — on macOS /tmp is a symlink to /private/tmp
    let workdir_matches = repo.workdir().map(|w| {
        std::fs::canonicalize(w).ok() == std::fs::canonicalize(&cwd).ok()
    }).unwrap_or(false);
    let has_worktree_config = repo.config().ok()
        .and_then(|c| c.get_string("core.worktree").ok()).is_some();
    if !workdir_matches && has_worktree_config {
        bail!("this repository is already converted or has core.worktree set");
    }

    // Guard: clean working tree
    if repo::is_working_tree_dirty(&repo) {
        bail!("working tree is dirty — commit or stash changes first");
    }

    // Guard: HEAD exists (at least one commit)
    let head = repo.head().context("no commits yet — create an initial commit first")?;
    let branch = head
        .shorthand()
        .context("HEAD does not point to a branch")?
        .to_string();

    let branch_dir = cwd.join(&branch);
    if branch_dir.exists() {
        bail!("directory '{}' already exists", branch);
    }

    // Get all working tree entries (files and dirs to move)
    let entries: Vec<PathBuf> = std::fs::read_dir(&cwd)
        .context("failed to read directory")?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name != ".git" && name != branch
        })
        .collect();

    eprintln!("Converting to branch-as-a-folder layout...");
    eprintln!("  branch: {}", branch);
    eprintln!("  moving {} entries into {}/", entries.len(), branch);

    // Create the branch directory and move files
    std::fs::create_dir(&branch_dir)
        .with_context(|| format!("failed to create directory '{}'", branch_dir.display()))?;

    for entry in &entries {
        let name = entry.file_name().unwrap();
        let dest = branch_dir.join(name);
        std::fs::rename(entry, &dest)
            .with_context(|| format!("failed to move '{}'", entry.display()))?;
    }

    // Set workdir and create gitlink
    repo.set_workdir(&branch_dir, true)
        .context("failed to set workdir")?;

    eprintln!("Converted: {}/", cwd.display());
    Ok(())
}
