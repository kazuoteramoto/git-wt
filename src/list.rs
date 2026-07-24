use anyhow::{Context, Result};
use git2::{Repository, StatusOptions};
use std::io::IsTerminal;

use crate::repo;

pub fn run(verbose: bool, color_flag: &str) -> Result<()> {
    let repo = repo::open_repo_from_cwd()?;
    let base = repo::base_dir(&repo);
    let use_color = color_enabled(&repo, color_flag);

    // Detect which branch we're standing in
    let current_branch = detect_current_branch();

    // Collect (name, path, opened_repo) for all checkouts
    struct Entry {
        name: String,
        path: String,
        repo: Repository,
    }
    let mut entries: Vec<Entry> = Vec::new();

    // Primary checkout
    if !repo.is_bare() {
        if let Some(workdir) = repo.workdir() {
            if let Ok(wt_repo) = Repository::open(workdir) {
                let name = wt_repo
                    .head()
                    .ok()
                    .and_then(|h| h.shorthand().map(|s| s.to_string()))
                    .unwrap_or_else(|| "(unknown)".to_string());
                let path = workdir
                    .strip_prefix(base)
                    .ok()
                    .map(|p| format!("{}/", p.display()))
                    .unwrap_or_else(|| format!("{}/", name));
                entries.push(Entry {
                    name,
                    path,
                    repo: wt_repo,
                });
            }
        }
    }

    // Linked worktrees
    let worktree_names = repo.worktrees().context("failed to list worktrees")?;
    for wt_name in worktree_names.iter() {
        let wt_name = wt_name.context("invalid worktree name")?;
        if let Ok(wt) = repo.find_worktree(wt_name) {
            if let Ok(wt_repo) = Repository::open_from_worktree(&wt) {
                // Use branch name from HEAD (may differ from worktree name, e.g. feat/test → feat-test)
                let name = wt_repo
                    .head()
                    .ok()
                    .and_then(|h| h.shorthand().map(|s| s.to_string()))
                    .unwrap_or_else(|| wt_name.to_string());
                let path = wt.path()
                    .strip_prefix(base)
                    .ok()
                    .map(|p| format!("{}/", p.display()))
                    .unwrap_or_else(|| format!("{}/", wt_name));
                entries.push(Entry {
                    name,
                    path,
                    repo: wt_repo,
                });
            }
        }
    }

    // Sort: current branch first, then alphabetical (matching git branch)
    entries.sort_by(|a, b| {
        let a_cur = current_branch.as_deref() == Some(&a.name);
        let b_cur = current_branch.as_deref() == Some(&b.name);
        b_cur
            .cmp(&a_cur)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    // Compute max branch name width for column alignment
    let max_name_width = entries
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(0);

    let tty = std::io::stdout().is_terminal();

    for entry in &entries {
        let is_current = current_branch.as_deref() == Some(&entry.name);
        let marker = color(
            if is_current { "*" } else { " " },
            Color::Green,
            use_color,
            tty,
        );
        let branch = color(
            &format!("{:<width$}", entry.name, width = max_name_width),
            if is_current { Color::Green } else { Color::None },
            use_color,
            tty,
        );

        let (sha, message) = head_info(&entry.repo);
        let short_sha = color(&short_sha_str(&entry.repo, &sha), Color::Yellow, use_color, tty);

        // Format: "marker branch sha message" (space-separated, aligned)
        let mut line = format!("{} {} {} {}", marker, branch, short_sha, message);

        if verbose {
            let (dirty, status) = dirty_status(&entry.repo);
            let status_colored = color(
                status,
                if dirty { Color::Red } else { Color::Green },
                use_color,
                tty,
            );
            line.push_str(&format!(" {} {}", entry.path, status_colored));
        }

        println!("{}", line);
    }

    Ok(())
}

// ─── helpers ────────────────────────────────────────────────────────

enum Color {
    None,
    Green,
    Red,
    Yellow,
}

fn color(text: &str, c: Color, enabled: bool, _tty: bool) -> String {
    if !enabled {
        return text.to_string();
    }
    let code = match c {
        Color::None => return text.to_string(),
        Color::Green => "32",
        Color::Red => "31",
        Color::Yellow => "33",
    };
    format!("\x1b[{}m{}\x1b[m", code, text)
}

fn color_enabled(repo: &Repository, flag: &str) -> bool {
    match flag {
        "always" => true,
        "never" => false,
        _ => {
            let config_val = repo
                .config()
                .ok()
                .and_then(|c| c.get_bool("color.wt").ok())
                .or_else(|| {
                    repo.config()
                        .ok()
                        .and_then(|c| c.get_bool("color.ui").ok())
                });
            let auto = config_val.unwrap_or(true);
            auto && std::io::stdout().is_terminal()
        }
    }
}

/// Detect which branch (if any) the user is currently standing inside.
fn detect_current_branch() -> Option<String> {
    let repo = Repository::discover(".").ok()?;
    let head = repo.head().ok()?;
    let branch = head.shorthand().map(|s| s.to_string());
    branch
}

/// Get (full_sha, message) from a repo's HEAD.
fn head_info(repo: &Repository) -> (String, String) {
    let head = match repo.head() {
        Ok(h) => h,
        Err(_) => return ("-".to_string(), "(no commits)".to_string()),
    };
    let sha = head
        .target()
        .map(|o| o.to_string())
        .unwrap_or_else(|| "-".to_string());
    let message = head
        .peel_to_commit()
        .ok()
        .and_then(|c| c.summary().map(|s| s.to_string()))
        .unwrap_or_else(|| "(no commits)".to_string());
    (sha, message)
}

/// Get an abbreviated SHA using git2's auto-sizing short_id.
fn short_sha_str(repo: &Repository, full_sha: &str) -> String {
    if full_sha == "-" {
        return "-".to_string();
    }
    let oid = match git2::Oid::from_str(full_sha) {
        Ok(o) => o,
        Err(_) => return full_sha[..7].to_string(),
    };
    repo.find_object(oid, None)
        .ok()
        .and_then(|obj| {
            obj.short_id()
                .ok()
                .and_then(|b| b.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| full_sha[..7].to_string())
}

/// Check if a worktree has uncommitted changes. Returns (is_dirty, status_label).
fn dirty_status(repo: &Repository) -> (bool, &'static str) {
    match repo.statuses(Some(
        StatusOptions::new()
            .include_untracked(true)
            .include_ignored(false),
    )) {
        Ok(statuses) if !statuses.is_empty() => (true, "dirty"),
        _ => (false, "clean"),
    }
}
