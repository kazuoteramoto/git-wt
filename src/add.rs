use anyhow::{Context, Result};
use git2::{BranchType, WorktreeAddOptions};

use crate::repo;

pub fn run(branch: &str, print_path: bool) -> Result<()> {
    let repo = repo::open_repo_from_cwd()?;
    let base_dir = repo::base_dir(&repo);

    let branch_exists = repo.find_branch(branch, BranchType::Local).is_ok();

    if !branch_exists {
        let head = repo
            .head()
            .context("no commits yet — create an initial commit first")?;
        let head_commit = head
            .peel_to_commit()
            .context("HEAD does not point to a commit")?;

        repo.branch(branch, &head_commit, false)
            .with_context(|| format!("failed to create branch '{}'", branch))?;
    }

    let worktree_path = base_dir.join(branch);

    // git2 creates the checkout dir itself, but needs parent dirs for
    // nested paths like feat/test → base_dir/feat/test/
    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory '{}'", parent.display()))?;
    }

    let wt_name = repo::worktree_name(branch);

    let mut wt_opts = WorktreeAddOptions::new();
    let branch_ref = repo
        .find_reference(&format!("refs/heads/{}", branch))
        .with_context(|| format!("failed to find branch ref for '{}'", branch))?;
    wt_opts.reference(Some(&branch_ref));

    repo.worktree(&wt_name, &worktree_path, Some(&wt_opts))
        .with_context(|| {
            format!(
                "failed to create worktree at '{}' — does it already exist?",
                worktree_path.display()
            )
        })?;

    if print_path {
        let canonical = worktree_path.canonicalize().unwrap_or(worktree_path);
        println!("{}", canonical.display());
    }

    Ok(())
}
