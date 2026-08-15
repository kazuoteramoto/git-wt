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

## Use Cases

- **Hotfix without stashing** — Ship an urgent fix in its own worktree without disturbing your WIP.
- **Review PRs side-by-side** — Check out a PR branch into its own folder and compare code next to `main`.
- **Run long tasks while you keep coding** — Build, test, or watch in one worktree while you continue editing in another.
- **Parallel feature work** — Work on multiple features at the same time, each in its own self-contained folder.
- **Clean branch lifecycle** — `git wt add` to start, `git wt rm` to clean up the branch, worktree, and directory in one step.

## Installation

### Homebrew (macOS and Linux)

```bash
brew tap kazuoteramoto/git-wt
brew install kazuoteramoto/git-wt/git-wt
```

### From source

```bash
cargo install --path .
```

### From GitHub Releases

Pre-built binaries are available on the [Releases page](https://github.com/kazuoteramoto/git-wt/releases).
Download the binary for your platform, rename it to `git-wt`, and make it executable:

```bash
# Example for macOS ARM (Apple Silicon)
curl -L -o git-wt https://github.com/kazuoteramoto/git-wt/releases/latest/download/git-wt-darwin-arm64
chmod +x git-wt
sudo mv git-wt /usr/local/bin/
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

### `git wt convert`

Convert an existing plain git repo to the branch-as-a-folder layout in-place.

```bash
cd my-existing-repo
git wt convert
# → .git stays at root, all files move into main/
```

The working tree must be clean (no uncommitted changes). The checkout directory is named after the current branch. Refuses if already converted, bare, on a detached HEAD, or inside a linked worktree.

### `git wt clone <url> [<dir>] [-b <branch>] [--depth <n>] [-o <name>]`

Clone a repository into a worktree-managed layout.

```bash
git wt clone https://github.com/user/repo.git
git wt clone https://github.com/user/repo.git my-fork
git wt clone https://github.com/user/repo.git --depth 1 -b develop
```

Creates `dir/{.git, <default-branch>/}`. The default branch is detected automatically; `-b` checks out another branch, `--depth` makes a shallow clone, `-o` names the remote (default `origin`). Unknown flags are rejected with an error — other `git clone` flags are not supported. Note that unlike `git clone`, `--depth` still fetches all branches (not just the default one); tags are fetched as with `git clone`.

For SSH URLs, authentication tries the ssh-agent first, then the default key files. A passphrase-protected key prompts interactively (hidden input, up to 3 attempts) — no flags needed.

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

Refuses to remove the primary checkout. Refuses unclean worktrees (modified or untracked files) unless `-f` is used, like `git worktree remove`. Checks merge status against the remote's default branch (falls back to local `main` or `master` if no remote HEAD is cached). Locked worktrees must be unlocked with `git worktree unlock` first.

### `git wt list [-v|--verbose] [--color=auto|always|never]`

List all worktrees in a format in the style of `git branch -v`. Space-aligned columns.

```bash
git wt list
# *  main    7e27e8b  initial commit
#    feature a1b2c3d  WIP: awesome stuff

git wt list -v                        # adds path and status columns
# *  main    7e27e8b  initial commit     main/     clean
#    feature a1b2c3d  WIP: awesome stuff feature/  dirty
```

The `*` marks the worktree you're currently inside. Colors are in the style of `git branch -v`: current branch green, SHA yellow, dirty red, clean green (git itself colors only branch names). Respects `color.ui` and `color.wt` git config. Use `--color=never` to disable, `--color=always` to force.

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

## Limitations

- **SSH keys** — `git wt clone` over SSH tries the ssh-agent first, then falls back to the default key files (`~/.ssh/id_ed25519`, `id_ecdsa`, `id_rsa`). Passphrase-protected keys work: they prompt interactively, with 3 attempts like ssh. Keys configured via `ssh_config` (`IdentityFile`) or custom paths are not supported; prompt input requires a terminal.
- **Empty remotes** — cloning an empty repository creates the layout with an unborn default branch; the worktree is populated by your first commit.
- **Locked worktrees** — `git wt rm` cannot remove a locked worktree; unlock it with `git worktree unlock` first.
