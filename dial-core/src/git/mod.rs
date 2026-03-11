use crate::errors::{DialError, Result};
use std::process::Command;

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
