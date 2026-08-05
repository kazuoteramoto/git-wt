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

/// Try to load an SSH key from the standard default paths.
///
/// Probes `~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa` in order, matching
/// OpenSSH's priority.  Returns `None` when no key file exists or the
/// file can't be loaded (e.g. encrypted with a passphrase).
fn try_default_ssh_key(user: &str, home: &str) -> Option<Cred> {
    for key_name in &["id_ed25519", "id_ecdsa", "id_rsa"] {
        let key_path = std::path::PathBuf::from(home)
            .join(".ssh")
            .join(key_name);
        if key_path.exists() {
            if let Ok(cred) = Cred::ssh_key(user, None, &key_path, None) {
                return Some(cred);
            }
        }
    }
    None
}

/// Create remote callbacks with SSH authentication.
///
/// First tries the SSH agent (`ssh-add -l`). If that fails (e.g. no agent
/// running), falls back to probing default key files: `~/.ssh/id_ed25519`,
/// `~/.ssh/id_ecdsa`, `~/.ssh/id_rsa`.
pub fn remote_callbacks() -> RemoteCallbacks<'static> {
    let mut cb = RemoteCallbacks::new();
    let mut tried_agent = false;
    cb.credentials(move |_url, username, allowed| {
        let user = username.unwrap_or("git");

        // Only try the agent once — if it fails, move on.
        if !tried_agent {
            tried_agent = true;
            return Cred::ssh_key_from_agent(user);
        }

        // Agent failed (or no agent running).  Probe default key files,
        // matching the order that OpenSSH uses.
        if allowed.contains(git2::CredentialType::SSH_KEY) {
            let home = match std::env::var("HOME") {
                Ok(h) => h,
                Err(_) => {
                    return Err(git2::Error::new(
                        git2::ErrorCode::Auth,
                        git2::ErrorClass::Ssh,
                        "HOME environment variable not set — cannot locate SSH keys",
                    ));
                }
            };
            if let Some(cred) = try_default_ssh_key(user, &home) {
                return Ok(cred);
            }
        }

        // Nothing worked — let libgit2 produce a descriptive error.
        Cred::ssh_key_from_agent(user)
    });
    cb
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
/// Branch names with / (e.g. feat/test) would create nested directories
/// under .git/worktrees/ that git can't enumerate, so replace / with -.
/// Derive a flat worktree name from a branch name.
/// Branch names with / (e.g. feat/test) would create nested directories
/// under .git/worktrees/ that git can't enumerate, so replace / with -.
pub fn worktree_name(branch: &str) -> String {
    branch.replace('/', "-")
}
