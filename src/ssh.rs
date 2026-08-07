use git2::{Cred, RemoteCallbacks};
use ssh_key::PrivateKey;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// Per-fetch passphrase state, captured by the credentials closure.
///
/// The callback is invoked once for the probe, then re-invoked whenever
/// libgit2's transport reports a rejected auth attempt (GIT_EAUTH).
/// Re-entry only happens when the *server* rejects a credential that the
/// transport could parse — a wrong passphrase on an encrypted key fails
/// earlier, at key-file parse time, and aborts the fetch outright without
/// re-entering the callback.
#[derive(Default)]
struct KeyState {
    /// We already handed libgit2 a default-key credential.
    probed: bool,
    /// Path of the key in play, for naming it in prompts and errors.
    last_key: Option<PathBuf>,
    /// Passphrase prompt attempts so far (max 3 — OpenSSH parity).
    tries: u32,
}

/// Detect whether a private key file is passphrase-encrypted, by reading
/// its header.  OpenSSH-format keys go through the `ssh-key` crate; legacy
/// PEM (DEK-Info / Proc-Type: 4,ENCRYPTED) and PKCS#8 (ENCRYPTED PRIVATE
/// KEY) are sniffed, since `ssh-key` can't parse those yet.
fn key_is_encrypted(path: &Path) -> bool {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return false,
    };
    let head = String::from_utf8_lossy(&data[..data.len().min(256)]);
    if head.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----") {
        return PrivateKey::from_openssh(&data).map_or(false, |k| k.is_encrypted());
    }
    head.contains("DEK-Info")
        || head.contains("Proc-Type: 4,ENCRYPTED")
        || head.contains("BEGIN ENCRYPTED PRIVATE KEY")
}

/// Check a passphrase against an encrypted key.  OpenSSH-format keys are
/// validated in-process via `ssh-key` decryption; legacy PEM keys fall
/// back to `ssh-keygen -y` (re-derives the public key, fails on a wrong
/// passphrase).  Returns `None` when validation can't run — then the
/// passphrase is passed through unvalidated and libssh2 will reject it
/// during auth instead.
fn validate_passphrase(key: &Path, passphrase: &str) -> Option<bool> {
    let data = std::fs::read(key).ok()?;
    if !data.starts_with(b"-----BEGIN OPENSSH PRIVATE KEY-----") {
        return std::process::Command::new("ssh-keygen")
            .args(["-y", "-P", passphrase, "-f"])
            .arg(key)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .ok()
            .map(|s| s.success());
    }
    let pk = PrivateKey::from_openssh(&data).ok()?;
    if !pk.is_encrypted() {
        return Some(true); // unencrypted — any passphrase is fine
    }
    Some(pk.decrypt(passphrase).is_ok())
}

/// Prompt for a passphrase and validate it against the key, up to 3
/// attempts (OpenSSH parity).  Validation matters because a wrong
/// passphrase surfaces as a key-file parse error that aborts the fetch
/// immediately — there is no chance to retry inside the callback.
fn validated_passphrase(
    key: &Path,
    tries: &mut u32,
    prompt: &mut dyn FnMut(&Path) -> Result<String, git2::Error>,
) -> Result<String, git2::Error> {
    loop {
        *tries += 1;
        if *tries > 3 {
            return Err(git2::Error::new(
                git2::ErrorCode::Auth,
                git2::ErrorClass::Ssh,
                format!(
                    "wrong passphrase for key '{}' after 3 attempts",
                    key.display()
                ),
            ));
        }
        let passphrase = prompt(key)?;
        if validate_passphrase(key, &passphrase) != Some(false) {
            return Ok(passphrase);
        }
    }
}

/// Decide what the credentials callback returns after the agent path failed.
///
/// The first call probes the default keys in OpenSSH priority order.  An
/// encrypted key triggers an immediate passphrase prompt (validated, up to
/// 3 attempts) before the credential is handed over, because a wrong
/// passphrase fails at key-file parse time and aborts the fetch without
/// any chance to retry inside the callback.
///
/// Any later call is a re-entry after the *server* rejected our credential.
/// Nothing left to try — the key parsed fine (or was unencrypted), so a
/// passphrase cannot help; return a descriptive error, which also
/// terminates libgit2's otherwise unbounded auth-retry loop.
fn default_key_cred(
    user: &str,
    home: &str,
    state: &mut KeyState,
    prompt: &mut dyn FnMut(&Path) -> Result<String, git2::Error>,
) -> Result<Cred, git2::Error> {
    if !state.probed {
        for key_name in &["id_ed25519", "id_ecdsa", "id_rsa"] {
            let key_path = PathBuf::from(home).join(".ssh").join(key_name);
            if !key_path.exists() {
                continue;
            }
            state.probed = true;
            state.last_key = Some(key_path.clone());

            let passphrase = if key_is_encrypted(&key_path) {
                Some(validated_passphrase(&key_path, &mut state.tries, prompt)?)
            } else {
                None
            };
            if let Ok(cred) = Cred::ssh_key(user, None, &key_path, passphrase.as_deref()) {
                return Ok(cred);
            }
            return Err(git2::Error::new(
                git2::ErrorCode::Auth,
                git2::ErrorClass::Ssh,
                format!("cannot load SSH key '{}'", key_path.display()),
            ));
        }
        return Err(git2::Error::new(
            git2::ErrorCode::Auth,
            git2::ErrorClass::Ssh,
            "no SSH key found in ~/.ssh (and ssh-agent authentication failed)",
        ));
    }

    let name = state
        .last_key
        .as_deref()
        .map_or_else(|| "?".to_string(), |p| p.display().to_string());
    Err(git2::Error::new(
        git2::ErrorCode::Auth,
        git2::ErrorClass::Ssh,
        format!("server rejected key '{name}' — check that the key is authorized on the server"),
    ))
}

/// Prompt for a key passphrase, ssh-style, on stderr, with hidden input.
/// Non-TTY stdin fails with a clear error instead of prompting.
fn ssh_passphrase_prompt(path: &Path) -> Result<String, git2::Error> {
    if !std::io::stdin().is_terminal() {
        return Err(git2::Error::new(
            git2::ErrorCode::Auth,
            git2::ErrorClass::Ssh,
            "cannot prompt for SSH key passphrase — not a terminal",
        ));
    }
    eprint!("Enter passphrase for key '{}': ", path.display());
    let pass = rpassword::read_password().map_err(|e| {
        git2::Error::new(
            git2::ErrorCode::Auth,
            git2::ErrorClass::Ssh,
            format!("failed to read SSH key passphrase: {e}"),
        )
    })?;
    eprintln!(); // fresh line after the hidden entry
    Ok(pass)
}

/// Create remote callbacks with SSH authentication.
///
/// First tries the SSH agent.  If that fails (e.g. no agent running), falls
/// back to probing default key files: `~/.ssh/id_ed25519`, `~/.ssh/id_ecdsa`,
/// `~/.ssh/id_rsa`.  An encrypted key prompts interactively for its
/// passphrase (validated, up to 3 attempts, OpenSSH parity).
pub fn remote_callbacks() -> RemoteCallbacks<'static> {
    let mut cb = RemoteCallbacks::new();
    let mut tried_agent = false;
    let mut key_state = KeyState::default();
    let mut prompt = ssh_passphrase_prompt;
    cb.credentials(move |_url, username, allowed| {
        let user = username.unwrap_or("git");

        // Only try the agent once — if it fails, move on.
        if !tried_agent {
            tried_agent = true;
            return Cred::ssh_key_from_agent(user);
        }

        // Agent failed (or no agent running).  Probe default key files,
        // matching the order that OpenSSH uses.  Any re-entry after the
        // probe means the probed key was rejected, so this may prompt for
        // a passphrase (see `default_key_cred`).
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
            return default_key_cred(user, &home, &mut key_state, &mut prompt);
        }

        // Nothing worked — let libgit2 produce a descriptive error.
        Cred::ssh_key_from_agent(user)
    });
    cb
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// Generate an ed25519 key with the given passphrase (`""` = unencrypted).
    /// Requires ssh-keygen on PATH (ships by default on macOS and most
    /// Linux distros).
    fn gen_key(dir: &TempDir, pass: &str) -> PathBuf {
        std::fs::create_dir_all(dir.path().join(".ssh")).unwrap();
        let key = dir.path().join(".ssh").join("id_ed25519");
        let out = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", pass, "-f"])
            .arg(&key)
            .output()
            .expect("ssh-keygen not on PATH");
        assert!(
            out.status.success(),
            "ssh-keygen failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        key
    }

    /// Extract the error from a decision result (`Cred` has no `Debug`, so
    /// `unwrap_err` won't compile).
    fn expect_err(res: Result<Cred, git2::Error>) -> git2::Error {
        match res {
            Ok(_) => panic!("expected error, got credential"),
            Err(e) => e,
        }
    }

    /// No default keys → error, and the prompt must never be called.
    #[test]
    fn no_keys_errors_without_prompting() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".ssh")).unwrap(); // empty ~/.ssh
        let mut state = KeyState::default();
        let mut calls = 0;
        let mut prompt = |_path: &Path| {
            calls += 1;
            Ok("x".to_string())
        };
        let err = expect_err(default_key_cred(
            "git",
            dir.path().to_str().unwrap(),
            &mut state,
            &mut prompt,
        ));
        assert!(err.message().contains("no SSH key found"));
        assert_eq!(calls, 0);
    }

    /// An encrypted key prompts on the first call, naming the key, and the
    /// validated passphrase is used to build the credential.
    #[test]
    fn encrypted_key_prompts_and_validates() {
        let dir = TempDir::new().unwrap();
        let key = gen_key(&dir, "testpass");
        let mut state = KeyState::default();
        let prompts = std::cell::RefCell::new(Vec::new());
        let mut prompt = |p: &Path| {
            prompts.borrow_mut().push(p.to_path_buf());
            Ok("testpass".to_string())
        };

        default_key_cred(
            "git",
            dir.path().to_str().unwrap(),
            &mut state,
            &mut prompt,
        )
        .unwrap();
        assert_eq!(*prompts.borrow(), vec![key.clone()]);
        assert_eq!(state.tries, 1);
        assert_eq!(state.last_key.as_deref(), Some(key.as_path()));
    }

    /// An unencrypted key is handed over without any prompt.
    #[test]
    fn unencrypted_key_never_prompts() {
        let dir = TempDir::new().unwrap();
        gen_key(&dir, "");
        let mut state = KeyState::default();
        let mut prompts = 0;
        let mut prompt = |_p: &Path| {
            prompts += 1;
            Ok("x".to_string())
        };

        default_key_cred(
            "git",
            dir.path().to_str().unwrap(),
            &mut state,
            &mut prompt,
        )
        .unwrap();
        assert_eq!(prompts, 0);
        assert_eq!(state.tries, 0);
    }

    /// Exactly 3 prompts on wrong passphrases (rejected by ssh-keygen
    /// validation), then a descriptive error naming the key.  A wrong
    /// passphrase aborts the fetch at key-parse time, so validation inside
    /// the callback is the only place a retry is possible.
    #[test]
    fn three_wrong_passphrases_then_error() {
        let dir = TempDir::new().unwrap();
        gen_key(&dir, "right-pass");
        let mut state = KeyState::default();
        let mut prompts = 0;
        let mut prompt = |_p: &Path| {
            prompts += 1;
            Ok("wrong".to_string())
        };

        let err = expect_err(default_key_cred(
            "git",
            dir.path().to_str().unwrap(),
            &mut state,
            &mut prompt,
        ));
        assert_eq!(prompts, 3);
        assert!(err.message().contains("3 attempts"));
        assert!(err.message().contains("id_ed25519"));
    }

    /// A failed prompt (e.g. non-TTY stdin) propagates unchanged.
    #[test]
    fn prompt_failure_propagates() {
        let dir = TempDir::new().unwrap();
        gen_key(&dir, "testpass");
        let mut state = KeyState::default();
        let mut prompt = |_p: &Path| {
            Err(git2::Error::new(
                git2::ErrorCode::Auth,
                git2::ErrorClass::Ssh,
                "cannot prompt for SSH key passphrase — not a terminal",
            ))
        };
        let err = expect_err(default_key_cred(
            "git",
            dir.path().to_str().unwrap(),
            &mut state,
            &mut prompt,
        ));
        assert!(err.message().contains("not a terminal"));
        assert_eq!(state.tries, 1);
    }

    /// Re-entry after a successful probe means the server rejected the key —
    /// nothing left to prompt for; the re-entry errors out immediately
    /// (terminating libgit2's otherwise unbounded auth-retry loop).
    #[test]
    fn reentry_after_probe_errors_without_prompting() {
        let dir = TempDir::new().unwrap();
        let key = gen_key(&dir, "testpass");
        let mut state = KeyState::default();
        let mut prompts = 0;
        let mut prompt = |_p: &Path| {
            prompts += 1;
            Ok("testpass".to_string())
        };

        default_key_cred(
            "git",
            dir.path().to_str().unwrap(),
            &mut state,
            &mut prompt,
        )
        .unwrap();
        let err = expect_err(default_key_cred(
            "git",
            dir.path().to_str().unwrap(),
            &mut state,
            &mut prompt,
        ));
        assert_eq!(prompts, 1); // only the probe prompt, no re-entry prompt
        assert!(err.message().contains("server rejected key"));
        assert!(err.message().contains(&key.display().to_string()));
    }

    /// Encryption detection: OpenSSH-format keys with and without a
    /// passphrase must be told apart by header sniffing.
    #[test]
    fn key_is_encrypted_detection() {
        let dir = TempDir::new().unwrap();
        let encrypted = gen_key(&dir, "testpass");
        let plain = {
            // second key name would be probed later; put it elsewhere
            std::fs::create_dir_all(dir.path().join("plain")).unwrap();
            let out = Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(dir.path().join("plain").join("id_ed25519"))
                .output()
                .unwrap();
            assert!(out.status.success());
            dir.path().join("plain").join("id_ed25519")
        };
        assert!(key_is_encrypted(&encrypted));
        assert!(!key_is_encrypted(&plain));
    }

    /// Ground truth for the fixture: the generated key is genuinely
    /// passphrase-protected (`Cred::ssh_key` itself cannot verify this — it
    /// only stores the passphrase).
    #[test]
    fn fixture_key_requires_passphrase() {
        let dir = TempDir::new().unwrap();
        let key = gen_key(&dir, "testpass");
        let check = |pass: &str| {
            Command::new("ssh-keygen")
                .args(["-y", "-f"])
                .arg(&key)
                .args(["-P", pass])
                .output()
                .unwrap()
        };
        assert!(!check("").status.success()); // no passphrase: refused
        assert!(check("testpass").status.success()); // right passphrase: works
    }
}
