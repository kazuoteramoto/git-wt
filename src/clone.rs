use anyhow::{bail, Context, Result};
use git2::{build::CheckoutBuilder, FetchOptions, Repository, RepositoryInitOptions};
use std::path::{Path, PathBuf};

use crate::repo;

pub fn run(url: &str, dir: Option<&str>, git_flags: &[String]) -> Result<()> {
    let git_flags = repo::normalize_flags(git_flags);

    for flag in &git_flags {
        match *flag {
            "--bare" | "--separate-git-dir" | "--no-checkout" => {
                bail!("flag '{}' is not compatible with git wt clone", flag);
            }
            _ => {}
        }
    }

    let base_dir = determine_base_dir(url, dir)?;

    if base_dir.exists() {
        bail!("destination '{}' already exists", base_dir.display());
    }

    let target_branch = repo::extract_flag_value(&git_flags, &["--branch", "-b"]);
    let depth =
        repo::extract_flag_value(&git_flags, &["--depth"]).and_then(|v| v.parse::<i32>().ok());

    clone_with_separate_git_dir(url, &base_dir, target_branch.map(String::from), depth)?;

    eprintln!("Cloned into {}", base_dir.display());
    Ok(())
}

fn clone_with_separate_git_dir(
    url: &str,
    base_dir: &Path,
    target_branch: Option<String>,
    depth: Option<i32>,
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
            .remote("origin", url)
            .with_context(|| format!("failed to add remote 'origin' at '{}'", url))?;

        let mut fetch_opts = FetchOptions::new();
        fetch_opts.remote_callbacks(repo::remote_callbacks());
        if let Some(d) = depth {
            fetch_opts.depth(d);
        }

        let refspec = "refs/heads/*:refs/remotes/origin/*";
        remote
            .fetch(&[refspec], Some(&mut fetch_opts), None)
            .with_context(|| format!("failed to fetch from '{}'", url))?;

        // Remote is still connected after fetch — get default branch
        let default_branch_buf = remote
            .default_branch()
            .context("failed to determine default branch from origin — remote may be empty")?;

        let default_branch_str = default_branch_buf
            .as_str()
            .context("default branch name is not valid UTF-8")?;
        default_branch_str
            .strip_prefix("refs/heads/")
            .unwrap_or(default_branch_str)
            .to_string()
    };

    let effective_branch = target_branch.as_deref().unwrap_or(&fetched_default_branch);

    let remote_ref_name = format!("refs/remotes/origin/{}", effective_branch);
    let remote_ref = repo
        .find_reference(&remote_ref_name)
        .with_context(|| {
            format!(
                "remote branch '{}' not found — was '{}' fetched?",
                effective_branch, effective_branch
            )
        })?;

    let commit = remote_ref
        .peel_to_commit()
        .context("failed to resolve remote branch to commit")?;

    repo.branch(effective_branch, &commit, false)
        .with_context(|| format!("failed to create local branch '{}'", effective_branch))?;

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

    // Create refs/remotes/origin/HEAD symref so default_branch_remote()
    // can resolve it locally (for rm merge checks, etc.)
    repo.reference_symbolic(
        "refs/remotes/origin/HEAD",
        &format!("refs/remotes/origin/{}", effective_branch),
        true,
        "git wt clone",
    )
    .context("failed to set remote HEAD reference")?;

    eprintln!("Default branch: {}", effective_branch);
    Ok(())
}

fn determine_base_dir(url: &str, dir: Option<&str>) -> Result<PathBuf> {
    if let Some(d) = dir {
        return Ok(PathBuf::from(d));
    }
    let name = url.trim_end_matches('/').trim_end_matches(".git");
    let basename = name.rsplit('/').next().unwrap_or(name);
    if basename.is_empty() {
        bail!("could not determine directory name from URL '{}' — specify a directory", url);
    }
    Ok(PathBuf::from(basename))
}
