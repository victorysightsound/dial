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
        Err(DialError::GitError(format!("Failed to create checkpoint: {}", stderr)))
    }
}

/// Restore a checkpoint by popping the most recent stash, then resetting the index
/// so all changes are unstaged (matching pre-checkpoint state).
/// Returns `Ok(true)` on success, `Ok(false)` if there is no stash to pop.
pub fn checkpoint_restore() -> Result<bool> {
    if !git_is_repo() {
        return Err(DialError::NotGitRepo);
    }

    // Check if there are any stashes
    let list_result = Command::new("git")
        .args(["stash", "list"])
        .output()?;

    let stash_list = String::from_utf8_lossy(&list_result.stdout);
    if stash_list.trim().is_empty() {
        return Ok(false);
    }

    // First, clean any current changes so the pop doesn't conflict
    let _ = Command::new("git")
        .args(["checkout", "--", "."])
        .output();
    let _ = Command::new("git")
        .args(["clean", "-fd"])
        .output();

    let pop_result = Command::new("git")
        .args(["stash", "pop"])
        .output()?;

    if pop_result.status.success() {
        Ok(true)
    } else {
        let stderr = String::from_utf8_lossy(&pop_result.stderr).to_string();
        Err(DialError::GitError(format!("Failed to restore checkpoint: {}", stderr)))
    }
}

/// Drop the most recent stash entry (used after successful validation to discard the checkpoint).
/// Returns `Ok(true)` if a stash was dropped, `Ok(false)` if there is no stash.
pub fn checkpoint_drop() -> Result<bool> {
    if !git_is_repo() {
        return Err(DialError::NotGitRepo);
    }

    let list_result = Command::new("git")
        .args(["stash", "list"])
        .output()?;

    let stash_list = String::from_utf8_lossy(&list_result.stdout);
    if stash_list.trim().is_empty() {
        return Ok(false);
    }

    let drop_result = Command::new("git")
        .args(["stash", "drop"])
        .output()?;

    if drop_result.status.success() {
        Ok(true)
    } else {
        let stderr = String::from_utf8_lossy(&drop_result.stderr).to_string();
        Err(DialError::GitError(format!("Failed to drop checkpoint: {}", stderr)))
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

pub fn git_commit(message: &str) -> Result<Option<String>> {
    // Stage all changes
    let add_result = Command::new("git")
        .args(["add", "-A"])
        .output()?;

    if !add_result.status.success() {
        return Err(DialError::GitError("Failed to stage changes".to_string()));
    }

    // Commit
    let commit_result = Command::new("git")
        .args(["commit", "-m", message])
        .output()?;

    if !commit_result.status.success() {
        return Ok(None);
    }

    // Get commit hash
    let hash_result = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()?;

    if hash_result.status.success() {
        let hash = String::from_utf8_lossy(&hash_result.stdout).trim().to_string();
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
    let result = Command::new("git")
        .args(["diff"])
        .output()?;

    Ok(String::from_utf8_lossy(&result.stdout).to_string())
}

/// Return the current working tree diff stat (unstaged changes summary) as a String.
pub fn git_diff_stat() -> Result<String> {
    let result = Command::new("git")
        .args(["diff", "--stat"])
        .output()?;

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
        assert!(result.is_ok(), "git_diff_stat() should return Ok in a git repo");
    }

    #[test]
    fn test_git_diff_returns_string() {
        // Verify return type is String (may be empty if no unstaged changes)
        let diff = git_diff().unwrap();
        // diff is a valid String; it may be empty or non-empty depending on working tree state
        let _ = diff.len();
    }
}
