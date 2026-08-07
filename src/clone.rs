use anyhow::{bail, Context, Result};
use git2::{
    build::CheckoutBuilder, FetchOptions, RemoteCallbacks, Repository, RepositoryInitOptions,
};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::ssh;

pub fn run(
    url: &str,
    dir: Option<&str>,
    branch: Option<&str>,
    depth: Option<i32>,
    remote_name: &str,
) -> Result<()> {
    let base_dir = determine_base_dir(url, dir)?;

    if base_dir.exists() {
        bail!("destination '{}' already exists", base_dir.display());
    }

    clone_with_separate_git_dir(url, &base_dir, branch.map(String::from), depth, remote_name)?;

    eprintln!("Cloned into {}", base_dir.display());
    Ok(())
}

fn clone_with_separate_git_dir(
    url: &str,
    base_dir: &Path,
    target_branch: Option<String>,
    depth: Option<i32>,
    remote_name: &str,
) -> Result<()> {
    std::fs::create_dir_all(base_dir)
        .with_context(|| format!("failed to create directory '{}'", base_dir.display()))?;

    let git_dir = base_dir.join(".git");

    let mut init_opts = RepositoryInitOptions::new();
    init_opts.no_dotgit_dir(true);
    let repo = Repository::init_opts(&git_dir, &init_opts)
        .with_context(|| format!("failed to init repo at '{}'", git_dir.display()))?;

    // Fetch first (implicit connect with SSH auth callbacks),
    // then get the default branch from the still-connected remote.
    let fetched_default_branch = {
        let mut remote = repo
            .remote(remote_name, url)
            .with_context(|| format!("failed to add remote '{}' at '{}'", remote_name, url))?;

        let mut fetch_opts = FetchOptions::new();
        fetch_opts.remote_callbacks(clone_remote_callbacks());
        // git clone follows tags by default — match that
        fetch_opts.download_tags(git2::AutotagOption::Auto);
        if let Some(d) = depth {
            fetch_opts.depth(d);
        }

        let refspec = format!("refs/heads/*:refs/remotes/{remote_name}/*");
        remote
            .fetch(&[&refspec], Some(&mut fetch_opts), None)
            .with_context(|| format!("failed to fetch from '{}'", url))?;

        // Remote is still connected after fetch — get default branch
        remote
            .default_branch()
            .ok()
            .and_then(|buf| buf.as_str().ok().map(str::to_string))
            .map(|s| s.strip_prefix("refs/heads/").unwrap_or(&s).to_string())
    };

    // Empty remote (no branch refs fetched) → unborn-HEAD layout like
    // `git clone` on an empty repository
    let has_remote_refs = repo
        .references_glob(&format!("refs/remotes/{remote_name}/*"))
        .ok()
        .map(|mut it| it.next().is_some())
        .unwrap_or(false);
    if !has_remote_refs {
        let default = fetched_default_branch.unwrap_or_else(|| "main".to_string());
        empty_clone(&repo, base_dir, remote_name, &default)?;
        return Ok(());
    }

    let fetched_default_branch =
        fetched_default_branch.context("failed to determine default branch from remote")?;

    let effective_branch = target_branch.as_deref().unwrap_or(&fetched_default_branch);

    let remote_ref_name = remote_ref(remote_name, effective_branch);
    let fetched_ref = repo.find_reference(&remote_ref_name).with_context(|| {
        format!(
            "remote branch '{}' not found — was '{}' fetched?",
            effective_branch, effective_branch
        )
    })?;

    let commit = fetched_ref
        .peel_to_commit()
        .context("failed to resolve remote branch to commit")?;

    let mut branch = repo
        .branch(effective_branch, &commit, false)
        .with_context(|| format!("failed to create local branch '{}'", effective_branch))?;
    branch
        .set_upstream(Some(&format!("{remote_name}/{effective_branch}")))
        .with_context(|| format!("failed to set upstream tracking for '{}'", effective_branch))?;

    let worktree_path = base_dir.join(effective_branch);
    std::fs::create_dir_all(&worktree_path)
        .with_context(|| format!("failed to create directory '{}'", worktree_path.display()))?;

    repo.set_workdir(&worktree_path, true)
        .with_context(|| format!("failed to set workdir to '{}'", worktree_path.display()))?;

    repo.set_head(&format!("refs/heads/{}", effective_branch))
        .with_context(|| format!("failed to set HEAD to '{}'", effective_branch))?;

    let mut checkout_opts = CheckoutBuilder::new();
    checkout_opts.force();
    repo.checkout_head(Some(&mut checkout_opts))
        .with_context(|| "failed to checkout working tree")?;

    // Create refs/remotes/<remote>/HEAD symref so cached_default_branch()
    // can resolve it locally (for rm merge checks, etc.). Points at the
    // remote's actual default branch — like git clone, independent of -b.
    repo.reference_symbolic(
        &remote_ref(remote_name, "HEAD"),
        &remote_ref(remote_name, &fetched_default_branch),
        true,
        "git wt clone",
    )
    .context("failed to set remote HEAD reference")?;

    eprintln!("Default branch: {}", effective_branch);
    Ok(())
}

/// Empty remote: create the layout with an unborn HEAD on the default
/// branch, like `git clone` on an empty repository. No checkout — the
/// user's first commit populates the worktree.
fn empty_clone(
    repo: &git2::Repository,
    base_dir: &Path,
    remote_name: &str,
    default_branch: &str,
) -> Result<()> {
    let worktree_path = base_dir.join(default_branch);
    std::fs::create_dir_all(&worktree_path)
        .with_context(|| format!("failed to create directory '{}'", worktree_path.display()))?;

    repo.set_workdir(&worktree_path, true)
        .with_context(|| format!("failed to set workdir to '{}'", worktree_path.display()))?;
    repo.set_head(&format!("refs/heads/{}", default_branch))
        .with_context(|| format!("failed to set HEAD to '{}'", default_branch))?;

    eprintln!("Default branch: {} (unborn)", default_branch);
    eprintln!(
        "Warning: remote '{}' appears to be empty — no commits to check out",
        remote_name
    );
    Ok(())
}

/// Build a remote-tracking ref path: `refs/remotes/<remote>/<name>`.
fn remote_ref(remote: &str, name: &str) -> String {
    format!("refs/remotes/{}/{}", remote, name)
}

fn determine_base_dir(url: &str, dir: Option<&str>) -> Result<PathBuf> {
    if let Some(d) = dir {
        return Ok(PathBuf::from(d));
    }
    let name = url.trim_end_matches('/').trim_end_matches(".git");
    let basename = name.rsplit('/').next().unwrap_or(name);
    if basename.is_empty() {
        bail!(
            "could not determine directory name from URL '{}' — specify a directory",
            url
        );
    }
    Ok(PathBuf::from(basename))
}

/// Render one fetch-progress update, or `None` when nothing should be printed.
fn fetch_progress_line(
    received: usize,
    total: usize,
    bytes: usize,
    idx_deltas: usize,
    tot_deltas: usize,
) -> Option<String> {
    if total == 0 {
        return None;
    }
    if received < total {
        let pct = received * 100 / total;
        return Some(format!(
            "\rReceiving objects: {}% ({}/{total}), {:.1} MiB",
            pct,
            received,
            bytes as f64 / (1024.0 * 1024.0)
        ));
    }
    if tot_deltas > 0 && idx_deltas < tot_deltas {
        let pct = idx_deltas * 100 / tot_deltas;
        return Some(format!(
            "\rResolving deltas: {}% ({idx_deltas}/{tot_deltas})",
            pct
        ));
    }
    Some(", done.".to_string())
}

fn clone_remote_callbacks() -> RemoteCallbacks<'static> {
    let mut cb = ssh::remote_callbacks();
    let tty = std::io::stderr().is_terminal();
    let mut finished = false;
    cb.transfer_progress(move |p| {
        if tty && !finished {
            if let Some(line) = fetch_progress_line(
                p.received_objects() as usize,
                p.total_objects() as usize,
                p.received_bytes(),
                p.indexed_deltas() as usize,
                p.total_deltas() as usize,
            ) {
                eprint!("{line}");
                if line == ", done." {
                    finished = true;
                }
            }
        }
        true
    });
    cb.sideband_progress(move |data| {
        if tty {
            eprint!("\rremote: {}", String::from_utf8_lossy(data));
        }
        true
    });
    cb
}

#[cfg(test)]
mod tests {
    use super::fetch_progress_line;

    #[test]
    fn empty_repo_is_silent() {
        assert_eq!(fetch_progress_line(0, 0, 0, 0, 0), None);
    }

    #[test]
    fn receiving_objects() {
        let line = fetch_progress_line(50, 100, 5_242_880, 0, 0).unwrap();
        assert!(line.contains("Receiving objects: 50% (50/100)"));
        assert!(line.contains("5.0 MiB"));
    }

    #[test]
    fn resolving_deltas() {
        let line = fetch_progress_line(100, 100, 0, 5, 10).unwrap();
        assert!(line.contains("Resolving deltas: 50% (5/10)"));
    }

    #[test]
    fn done_no_deltas() {
        assert_eq!(fetch_progress_line(100, 100, 0, 0, 0).unwrap(), ", done.");
    }

    #[test]
    fn done_after_deltas() {
        assert_eq!(fetch_progress_line(100, 100, 0, 10, 10).unwrap(), ", done.");
    }
}
