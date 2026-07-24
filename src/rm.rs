use anyhow::{bail, Context, Result};
use git2::BranchType;

use crate::repo;

pub fn run(branch: &str, force: bool) -> Result<()> {
    let repo = repo::open_repo_from_cwd()?;

    // 1. Protect primary checkout from removal
    if !repo.is_bare() {
        if let Ok(head) = repo.head() {
            if let Some(head_branch) = head.shorthand() {
                if head_branch == branch {
                    bail!(
                        "cannot remove '{}' — it is the primary checkout (HEAD).\n\
                         To remove it, clone again or change the primary checkout branch.",
                        branch
                    );
                }
            }
        }
    }

    // 2. Find the linked worktree (name may differ from branch if branch has /)
    let wt_name = repo::worktree_name(branch);
    let wt = repo
        .find_worktree(&wt_name)
        .with_context(|| format!("no worktree for branch '{}'", branch))?;

    // 3. Determine default branch for merge check
    let default_branch = repo::cached_default_branch(&repo)
        .or_else(|_| repo::default_branch_local(&repo))
        .context("cannot determine default branch for merge check")?;

    // 4. Check merge status
    let branch_ref = repo
        .find_branch(branch, BranchType::Local)
        .with_context(|| format!("branch '{}' not found", branch))?;
    let branch_commit = branch_ref
        .get()
        .peel_to_commit()
        .context("failed to resolve branch to commit")?;
    let branch_oid = branch_commit.id();

    let default_ref = repo
        .find_branch(&default_branch, BranchType::Local)
        .with_context(|| format!("default branch '{}' not found locally", default_branch))?;
    let default_commit = default_ref
        .get()
        .peel_to_commit()
        .context("failed to resolve default branch to commit")?;
    let default_oid = default_commit.id();

    let is_merged = branch_oid == default_oid
        || repo
            .graph_descendant_of(default_oid, branch_oid)
            .unwrap_or(false);

    if !is_merged && !force {
        let (ahead, _behind) = repo
            .graph_ahead_behind(branch_oid, default_oid)
            .unwrap_or((0, 0));

        eprintln!(
            "error: branch '{}' is not fully merged into '{}' ({} unmerged commit(s))",
            branch, default_branch, ahead
        );
        eprintln!("If you are sure, use: git wt rm -f {}", branch);
        bail!("branch '{}' is not fully merged into '{}'", branch, default_branch);
    }

    // 5. Prune worktree (removes metadata + working tree directory)
    let mut prune_opts = git2::WorktreePruneOptions::new();
    prune_opts.valid(true);
    prune_opts.working_tree(true);
    wt.prune(Some(&mut prune_opts))
        .with_context(|| format!("failed to remove worktree '{}'", branch))?;

    // 6. Delete the local branch
    let mut branch_obj = repo
        .find_branch(branch, BranchType::Local)
        .with_context(|| format!("branch '{}' not found for deletion", branch))?;
    branch_obj
        .delete()
        .with_context(|| format!("failed to delete branch '{}'", branch))?;

    eprintln!("Removed worktree and branch '{}'", branch);
    Ok(())
}
