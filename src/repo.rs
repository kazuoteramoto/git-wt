use anyhow::{Context, Result};
use git2::{BranchType, Repository};
use std::path::Path;

/// Open the common repository from cwd, following worktree links.
pub fn open_repo_from_cwd() -> Result<Repository> {
    let repo =
        Repository::discover(".").context("not in a git repository — run this from a worktree")?;

    if repo.is_worktree() {
        let worktree_gitdir = repo.path().to_path_buf();
        let commondir_path = worktree_gitdir.join("commondir");
        if commondir_path.exists() {
            let commondir = std::fs::read_to_string(&commondir_path)
                .context("failed to read commondir file")?;
            let commondir = commondir.trim();
            return Repository::open(commondir)
                .with_context(|| format!("failed to open common git dir at '{}'", commondir));
        }
    }

    Ok(repo)
}

/// Get the base directory (parent of the common .git dir).
pub fn base_dir(repo: &Repository) -> &Path {
    repo.path().parent().expect("git dir has no parent")
}

/// Determine the default branch from the locally-cached remote HEAD symref.
/// No network access — the symref is set during clone/fetch.
///
/// Tries `origin` first (standard git convention), then falls back to any
/// `refs/remotes/*/HEAD` reference to support custom remote names
/// (e.g. `git wt clone -- --origin upstream <url>`).
pub fn cached_default_branch(repo: &Repository) -> Result<String> {
    // Try origin first — deterministic, matches git convention
    if let Ok(head_ref) = repo.find_reference("refs/remotes/origin/HEAD") {
        if let Ok(Some(target)) = head_ref.symbolic_target() {
            let branch = target
                .strip_prefix("refs/remotes/origin/")
                .unwrap_or(&target)
                .to_string();
            return Ok(branch);
        }
    }

    // Fallback: enumerate any remote HEAD (for custom --origin names)
    let mut refs = repo
        .references_glob("refs/remotes/*/HEAD")
        .context("failed to enumerate remote references")?;

    for head_ref_result in &mut refs {
        let head_ref = head_ref_result.context("failed to iterate remote references")?;
        if let Ok(Some(target)) = head_ref.symbolic_target() {
            // Derive the prefix from the HEAD ref name:
            //   "refs/remotes/upstream/HEAD" → "refs/remotes/upstream/"
            // Then strip it from the target to get the branch name, which
            // correctly handles nested names like "feature/foo".
            let head_name = head_ref.name().unwrap_or("");
            let remote_prefix = head_name.strip_suffix("HEAD").unwrap_or(head_name);
            let branch = target
                .strip_prefix(remote_prefix)
                .unwrap_or(&target)
                .to_string();
            return Ok(branch);
        }
    }

    anyhow::bail!("cannot determine default branch — no remote HEAD found")
}

/// Fallback: try common default branch names locally.
pub fn default_branch_local(repo: &Repository) -> Result<String> {
    for name in &["main", "master"] {
        if repo.find_branch(name, BranchType::Local).is_ok() {
            return Ok(name.to_string());
        }
    }
    anyhow::bail!("cannot determine default branch — no 'main', 'master', or remote HEAD found")
}

/// Check whether the working tree has uncommitted changes.
pub fn is_working_tree_dirty(repo: &Repository) -> bool {
    repo.statuses(Some(
        git2::StatusOptions::new()
            .include_untracked(true)
            .include_ignored(false),
    ))
    .map(|s| !s.is_empty())
    .unwrap_or(true) // treat errors as dirty (safe for guards)
}
/// Derive a flat worktree name from a branch name.
/// Branch names with / (e.g. feat/test) would create nested directories
/// under .git/worktrees/ that git can't enumerate, so replace / with -.
pub fn worktree_name(branch: &str) -> String {
    branch.replace('/', "-")
}
