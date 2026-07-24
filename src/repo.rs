use anyhow::{Context, Result};
use git2::{BranchType, Cred, RemoteCallbacks, Repository};
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
pub fn cached_default_branch(repo: &Repository) -> Result<String> {
    // Read refs/remotes/origin/HEAD → points to e.g. refs/remotes/origin/main
    let head_ref = repo
        .find_reference("refs/remotes/origin/HEAD")
        .context("no cached remote HEAD — fetch from origin first")?;

    let target = head_ref
        .symbolic_target()
        .context("remote HEAD is not a symbolic reference")?;

    let branch = target
        .strip_prefix("refs/remotes/origin/")
        .unwrap_or(target)
        .to_string();

    Ok(branch)
}

/// Fallback: try common default branch names locally.
pub fn default_branch_local(repo: &Repository) -> Result<String> {
    for name in &["main", "master"] {
        if repo.find_branch(name, BranchType::Local).is_ok() {
            return Ok(name.to_string());
        }
    }
    anyhow::bail!("cannot determine default branch — no 'main', 'master', or origin HEAD found")
}

/// Normalize pass-through git flags: filter out bare `--` separators.
pub fn normalize_flags(flags: &[String]) -> Vec<&str> {
    flags
        .iter()
        .map(|s| s.as_str())
        .filter(|s| *s != "--")
        .collect()
}

/// Extract a value from pass-through flags (e.g., `--depth 1` → Some("1")).
pub fn extract_flag_value<'a>(args: &[&'a str], flag_names: &[&str]) -> Option<&'a str> {
    for i in 0..args.len() {
        if flag_names.contains(&args[i]) {
            return args.get(i + 1).copied();
        }
        for name in flag_names {
            if let Some(rest) = args[i].strip_prefix(&format!("{}=", name)) {
                return Some(rest);
            }
        }
    }
    None
}

/// Create remote callbacks with SSH agent authentication.
pub fn remote_callbacks() -> RemoteCallbacks<'static> {
    let mut cb = RemoteCallbacks::new();
    cb.credentials(|_url, username, _allowed| {
        Cred::ssh_key_from_agent(username.unwrap_or("git"))
    });
    cb
}

/// Derive a flat worktree name from a branch name.
/// Branch names with / (e.g. feat/test) would create nested directories
/// under .git/worktrees/ that git can't enumerate, so replace / with -.
pub fn worktree_name(branch: &str) -> String {
    branch.replace('/', "-")
}
