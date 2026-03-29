use crate::config::config_get;
use crate::errors::{DialError, Result};
use std::process::Command;

/// Check if checkpoints are enabled via the `enable_checkpoints` config key.
/// Defaults to true when the key is absent or not "false"/"0".
pub fn checkpoints_enabled() -> bool {
    match config_get("enable_checkpoints") {
        Ok(Some(val)) => !matches!(val.as_str(), "false" | "0"),
        _ => true,
    }
}

/// Create a checkpoint by stashing the current working tree (including untracked files).
/// Returns `Ok(true)` if a stash was created, `Ok(false)` if the tree was clean (no-op).
pub fn checkpoint_create(id: &str) -> Result<bool> {
    if !git_is_repo() {
        return Err(DialError::NotGitRepo);
    }

    if !git_has_changes() {
        return Ok(false);
    }

    let msg = format!("dial-checkpoint-{}", id);
    let result = Command::new("git")
        .args(["stash", "push", "-u", "-m", &msg])
        .output()?;

    if result.status.success() {
        Ok(true)
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        Err(DialError::GitError(format!(
            "Failed to create checkpoint: {}",
            stderr
        )))
    }
}

/// Restore a checkpoint by popping the most recent stash, then resetting the index
/// so all changes are unstaged (matching pre-checkpoint state).
/// Returns `Ok(true)` on success, `Ok(false)` if there is no stash to pop.
///
/// If `git stash pop` fails (e.g. merge conflict from manual commits between
/// iterate and validate), falls back to `git reset --hard HEAD` + `git stash drop`
/// to guarantee a clean working tree.
pub fn checkpoint_restore() -> Result<bool> {
    if !git_is_repo() {
        return Err(DialError::NotGitRepo);
    }

    // Check if there are any stashes
    let list_result = Command::new("git").args(["stash", "list"]).output()?;

    let stash_list = String::from_utf8_lossy(&list_result.stdout);
    if stash_list.trim().is_empty() {
        return Ok(false);
    }

    // First, clean any current changes so the pop doesn't conflict
    let _ = Command::new("git").args(["checkout", "--", "."]).output();
    let _ = Command::new("git").args(["clean", "-fd"]).output();

    let pop_result = Command::new("git").args(["stash", "pop"]).output()?;

    if pop_result.status.success() {
        return Ok(true);
    }

    // Stash pop failed (likely merge conflict). Recover by resetting to a clean
    // state and dropping the stash. The pre-checkpoint state is lost, but the
    // working tree is guaranteed clean for the next attempt.
    eprintln!(
        "Warning: checkpoint restore failed ({}), recovering with hard reset",
        String::from_utf8_lossy(&pop_result.stderr).trim()
    );

    // Abort any in-progress merge from the failed pop
    let _ = Command::new("git")
        .args(["reset", "--hard", "HEAD"])
        .output();
    let _ = Command::new("git").args(["clean", "-fd"]).output();

    // The failed pop leaves the stash in place — drop it
    let _ = Command::new("git").args(["stash", "drop"]).output();

    Ok(true)
}

/// Drop the most recent stash entry (used after successful validation to discard the checkpoint).
/// Returns `Ok(true)` if a stash was dropped, `Ok(false)` if there is no stash.
pub fn checkpoint_drop() -> Result<bool> {
    if !git_is_repo() {
        return Err(DialError::NotGitRepo);
    }

    let list_result = Command::new("git").args(["stash", "list"]).output()?;

    let stash_list = String::from_utf8_lossy(&list_result.stdout);
    if stash_list.trim().is_empty() {
        return Ok(false);
    }

    let drop_result = Command::new("git").args(["stash", "drop"]).output()?;

    if drop_result.status.success() {
        Ok(true)
    } else {
        let stderr = String::from_utf8_lossy(&drop_result.stderr).to_string();
        Err(DialError::GitError(format!(
            "Failed to drop checkpoint: {}",
            stderr
        )))
    }
}

pub fn git_is_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn git_has_changes() -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

/// Patterns that indicate potentially dangerous files (secrets, credentials, keys).
/// Checked before staging to prevent accidental commits of sensitive data.
const DANGEROUS_PATTERNS: &[&str] = &[
    ".env",
    ".env.local",
    ".env.production",
    "credentials.json",
    "service-account.json",
    ".pem",
    ".key",
    ".p12",
    ".pfx",
    "id_rsa",
    "id_ed25519",
    ".secret",
    ".secrets",
];

/// Check staged files for potentially dangerous filenames.
/// Returns a list of warnings (empty if all safe).
fn check_staged_for_secrets() -> Vec<String> {
    let output = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .output();

    let Ok(result) = output else {
        return vec![];
    };

    let files = String::from_utf8_lossy(&result.stdout);
    let mut warnings = Vec::new();

    for file in files.lines() {
        let lower = file.to_lowercase();
        for pattern in DANGEROUS_PATTERNS {
            if lower.ends_with(pattern) || lower.contains(&format!("/{}", pattern)) {
                warnings.push(file.to_string());
                break;
            }
        }
    }

    warnings
}

pub fn git_commit(message: &str) -> Result<Option<String>> {
    // Stage all changes
    let add_result = Command::new("git").args(["add", "-A"]).output()?;

    if !add_result.status.success() {
        return Err(DialError::GitError("Failed to stage changes".to_string()));
    }

    // Safety check: warn about potentially dangerous files before committing
    let dangerous = check_staged_for_secrets();
    if !dangerous.is_empty() {
        eprintln!(
            "Warning: potentially sensitive files staged for commit: {}",
            dangerous.join(", ")
        );
        eprintln!("Add these to .gitignore if they should not be committed.");
        // Unstage the dangerous files and continue with the rest
        for file in &dangerous {
            let _ = Command::new("git")
                .args(["reset", "HEAD", "--", file])
                .output();
        }
    }

    // Commit
    let commit_result = Command::new("git")
        .args(["commit", "-m", message])
        .output()?;

    if !commit_result.status.success() {
        return Ok(None);
    }

    // Get commit hash
    let hash_result = Command::new("git").args(["rev-parse", "HEAD"]).output()?;

    if hash_result.status.success() {
        let hash = String::from_utf8_lossy(&hash_result.stdout)
            .trim()
            .to_string();
        Ok(Some(hash))
    } else {
        Ok(None)
    }
}

pub fn git_revert_to(commit_hash: &str) -> Result<bool> {
    let result = Command::new("git")
        .args(["reset", "--hard", commit_hash])
        .output()?;

    Ok(result.status.success())
}

pub fn git_get_last_commit() -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Return the current working tree diff (unstaged changes) as a String.
pub fn git_diff() -> Result<String> {
    let result = Command::new("git").args(["diff"]).output()?;

    Ok(String::from_utf8_lossy(&result.stdout).to_string())
}

/// Return the current working tree diff stat (unstaged changes summary) as a String.
pub fn git_diff_stat() -> Result<String> {
    let result = Command::new("git").args(["diff", "--stat"]).output()?;

    Ok(String::from_utf8_lossy(&result.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_diff_returns_ok() {
        // In the project repo, git diff should succeed (returns Ok even if empty)
        let result = git_diff();
        assert!(result.is_ok(), "git_diff() should return Ok in a git repo");
    }

    #[test]
    fn test_git_diff_stat_returns_ok() {
        // In the project repo, git diff --stat should succeed
        let result = git_diff_stat();
        assert!(
            result.is_ok(),
            "git_diff_stat() should return Ok in a git repo"
        );
    }

    #[test]
    fn test_git_diff_returns_string() {
        // Verify return type is String (may be empty if no unstaged changes)
        let diff = git_diff().unwrap();
        // diff is a valid String; it may be empty or non-empty depending on working tree state
        let _ = diff.len();
    }

    #[test]
    fn test_dangerous_patterns_detects_env() {
        // .env should match DANGEROUS_PATTERNS
        let lower = ".env".to_lowercase();
        let matched = DANGEROUS_PATTERNS.iter().any(|p| lower.ends_with(p));
        assert!(matched, ".env should be flagged as dangerous");
    }

    #[test]
    fn test_dangerous_patterns_detects_pem() {
        let lower = "server.pem".to_lowercase();
        let matched = DANGEROUS_PATTERNS.iter().any(|p| lower.ends_with(p));
        assert!(matched, ".pem files should be flagged as dangerous");
    }

    #[test]
    fn test_dangerous_patterns_ignores_safe_files() {
        let safe_files = ["main.rs", "README.md", "Cargo.toml", "test.js"];
        for file in &safe_files {
            let lower = file.to_lowercase();
            let matched = DANGEROUS_PATTERNS.iter().any(|p| lower.ends_with(p));
            assert!(!matched, "{} should not be flagged as dangerous", file);
        }
    }

    #[test]
    fn test_dangerous_patterns_detects_key_files() {
        let dangerous = ["id_rsa", "id_ed25519", "private.key", "cert.p12"];
        for file in &dangerous {
            let lower = file.to_lowercase();
            let matched = DANGEROUS_PATTERNS.iter().any(|p| lower.ends_with(p));
            assert!(matched, "{} should be flagged as dangerous", file);
        }
    }
}
