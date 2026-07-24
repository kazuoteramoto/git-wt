# git-wt

Manage git worktrees with a branch-as-a-folder layout — every branch gets its own peer directory alongside the `.git` repo.

```text
myproject/
├── .git/         ← git repository (workdir = main/)
├── main/         ← primary checkout (what .git tracks)
├── feature-a/    ← linked worktree for feature-a
├── feature-b/    ← linked worktree for feature-b
└── ...
```

No files leak into the root directory. Each branch is a self-contained folder — you can open multiple terminals, run different branches, compare code side-by-side, or run tests in one branch while reviewing PRs in another.

## Installation

```bash
cargo install --path .
```

The binary is named `git-wt`. Git automatically discovers it as `git wt`.

## Quick Start

```bash
# Clone a repo into worktree layout
git wt clone https://github.com/user/repo.git

# Move into the main worktree
cd repo/main

# Start a new feature
git wt add my-feature
cd ../my-feature

# Work on the feature, commit, push...
echo "awesome" > feature.txt
git add . && git commit -m "Add feature"
git push -u origin my-feature

# When done, merge via PR on GitHub, then clean up locally
cd ../main
git pull origin main      # get the merge
git wt rm my-feature      # deletes worktree + branch

# List all your worktrees
git wt list
```

### Git commands from the root

The root directory (`myproject/`) is a valid git context. Since `.git` has `core.worktree` set to `main/`, running git commands from the root operates on the primary checkout:

```bash
cd myproject

# These all work from the root — no need to cd into main/
git status          # shows working tree status of main/
git fetch           # fetches into the shared .git repo
git branch          # lists all branches
git log             # shows history of main
```

This makes the root a natural "control center" for the whole project — do administrative git operations there, and switch into individual worktree directories for editing code.

## Commands

### `git wt clone <url> [<dir>] [-- <git-clone-flags>]`

Clone a repository into a worktree-managed layout.

```bash
git wt clone https://github.com/user/repo.git
git wt clone https://github.com/user/repo.git my-fork
git wt clone https://github.com/user/repo.git -- --depth 1 --branch develop
```

Creates `dir/{.git, <default-branch>/}`. The default branch is detected automatically.

Flags `--bare`, `--separate-git-dir`, and `--no-checkout` are rejected (they conflict with the layout).

### `git wt add <branch> [-p|--print-path]`

Create a new worktree for a branch. Creates the branch if it doesn't exist yet.

```bash
git wt add my-feature
git wt add my-feature -p          # prints the absolute path
```

Run from inside any existing worktree.

### `git wt rm <branch> [-f|--force]`

Remove a worktree and delete its branch.

```bash
git wt rm my-feature              # blocks if branch has unmerged commits
git wt rm -f my-feature           # force deletion even if unmerged
```

Refuses to remove the primary checkout. Checks merge status against the remote's default branch (falls back to local `main` or `master` if no remote HEAD is cached).

### `git wt list [-v|--verbose] [--color=auto|always|never]`

List all worktrees in a format matching `git branch -v`. Space-aligned columns.

```bash
git wt list
# *  main    7e27e8b  initial commit
#    feature a1b2c3d  WIP: awesome stuff

git wt list -v                        # adds path and status columns
# *  main    7e27e8b  initial commit     main/     clean
#    feature a1b2c3d  WIP: awesome stuff feature/  dirty
```

The `*` marks the worktree you're currently inside. Colors match `git branch -v` (* and current branch green, SHA yellow, dirty red, clean green). Respects `color.ui` and `color.wt` git config. Use `--color=never` to disable, `--color=always` to force.

## Shell Alias

For automatic `cd` into a new worktree, add to your shell config:

```bash
# bash / zsh
gwt() { cd "$(git wt add --print-path "$@")"; }

# fish
function gwt; cd (git wt add --print-path $argv); end
```

Then `gwt my-feature` creates the worktree AND switches you into it.

## Why not vanilla `git worktree`?

`git worktree` gives you parallel checkouts, but it doesn't help with:

- **Cleanup** — you need `git worktree remove` + `git branch -D` + `rm -rf` the directory. `git wt rm` does all three.
- **Layout** — vanilla worktrees scatter across your filesystem however you specified paths. `git wt` enforces the branch-as-a-folder peer layout.
- **Setup** — cloning with `--separate-git-dir` is an obscure flag. `git wt clone` handles it and sets up the root directory as a functional git context.
- **Visibility** — `git wt list` shows you all worktrees with dirty status and last commit, not just paths.
- **Safety** — `git wt rm` checks if your branch is merged before deleting it.
- **Root as control center** — run `git status`, `git fetch`, `git branch` from the project root without needing to `cd` into a worktree first.
