use crate::config::config_get;
use crate::errors::{DialError, Result};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::thread;
use std::time::Duration;

const INDEX_LOCK_RETRY_ATTEMPTS: usize = 8;
const INDEX_LOCK_RETRY_DELAY_MS: u64 = 250;
const STALE_INDEX_LOCK_AGE_SECS: u64 = 5;
const COMMIT_SUBJECT_LIMIT: usize = 72;
const COMMIT_ACTIONS: &[&str] = &[
    "add",
    "create",
    "document",
    "extend",
    "fix",
    "finish",
    "implement",
    "improve",
    "refactor",
    "remove",
    "rename",
    "update",
    "bump",
];
const TEST_SUFFIXES: &[&str] = &[
    " and add tests",
    " and add test",
    " and add unit tests",
    " and add integration tests",
];
const CLAUSE_SUFFIXES: &[&str] = &[" so ", " while "];

/// Check if checkpoints are enabled via the `enable_checkpoints` config key.
/// Defaults to true when the key is absent or not "false"/"0".
pub fn checkpoints_enabled() -> bool {
    match config_get("enable_checkpoints") {
        Ok(Some(val)) => !matches!(val.as_str(), "false" | "0"),
        _ => true,
    }
}

fn git_output(args: &[&str]) -> Result<Output> {
    Ok(Command::new("git").args(args).output()?)
}

fn git_dir_path() -> Option<PathBuf> {
    git_output(&["rev-parse", "--git-dir"])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

fn git_index_lock_path() -> Option<PathBuf> {
    git_dir_path().map(|git_dir| git_dir.join("index.lock"))
}

fn git_config_value(key: &str) -> Option<String> {
    git_output(&["config", "--get", key])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn git_head_author_field(format_arg: &str) -> Option<String> {
    git_output(&["log", "-1", format_arg])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn output_mentions_index_lock(output: &Output) -> bool {
    let stderr = String::from_utf8_lossy(&output.stderr);
    stderr.contains("index.lock")
}

fn output_error_message(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }

    format!("exit status {}", output.status)
}

fn ensure_git_commit_identity() -> Result<()> {
    let name = git_config_value("user.name");
    let email = git_config_value("user.email");
    if name.is_some() && email.is_some() {
        return Ok(());
    }

    let fallback_name = name.or_else(|| git_head_author_field("--format=%an"));
    let fallback_email = email.or_else(|| git_head_author_field("--format=%ae"));

    let (Some(name), Some(email)) = (fallback_name, fallback_email) else {
        return Err(DialError::GitError(
            "Git author identity is not configured. Run `git config user.name \"Your Name\"` and `git config user.email \"you@example.com\"` before validating or auto-running."
                .to_string(),
        ));
    };

    git_output_with_retry(&["config", "user.name", &name])?;
    git_output_with_retry(&["config", "user.email", &email])?;
    eprintln!(
        "Info: configured missing git author identity from the latest commit author: {} <{}>",
        name, email
    );
    Ok(())
}

fn clear_stale_index_lock(max_age: Duration) -> Result<bool> {
    let Some(lock_path) = git_index_lock_path() else {
        return Ok(false);
    };
    if !lock_path.exists() {
        return Ok(false);
    }

    let metadata = std::fs::metadata(&lock_path)?;
    let modified = metadata.modified()?;
    let Ok(age) = modified.elapsed() else {
        return Ok(false);
    };
    if age < max_age {
        return Ok(false);
    }

    std::fs::remove_file(&lock_path)?;
    eprintln!(
        "Warning: removed stale git index lock at {} after waiting {:.1}s",
        lock_path.display(),
        age.as_secs_f64()
    );
    Ok(true)
}

fn git_output_with_retry_policy(
    args: &[&str],
    retry_attempts: usize,
    retry_delay: Duration,
    stale_lock_age: Duration,
) -> Result<Output> {
    let attempts = retry_attempts.max(1);
    let mut last_output: Option<Output> = None;

    for attempt in 0..attempts {
        let output = git_output(args)?;
        if output.status.success() || !output_mentions_index_lock(&output) {
            return Ok(output);
        }
        last_output = Some(output);

        if attempt + 1 < attempts {
            thread::sleep(retry_delay.saturating_mul((attempt + 1) as u32));
        }
    }

    if clear_stale_index_lock(stale_lock_age)? {
        let output = git_output(args)?;
        if output.status.success() || !output_mentions_index_lock(&output) {
            return Ok(output);
        }
        last_output = Some(output);
    }

    let output = last_output.expect("index-lock retries should record a git failure");
    Err(DialError::GitError(format!(
        "git {} failed: {}",
        args.join(" "),
        output_error_message(&output)
    )))
}

fn git_output_with_retry(args: &[&str]) -> Result<Output> {
    git_output_with_retry_policy(
        args,
        INDEX_LOCK_RETRY_ATTEMPTS,
        Duration::from_millis(INDEX_LOCK_RETRY_DELAY_MS),
        Duration::from_secs(STALE_INDEX_LOCK_AGE_SECS),
    )
}

fn strip_task_prefixes(message: &str) -> String {
    let mut current = message.trim().to_string();

    if let Some(rest) = current.strip_prefix("- [ ]") {
        current = rest.trim().to_string();
    } else if let Some(rest) = current.strip_prefix("- [x]") {
        current = rest.trim().to_string();
    } else if let Some(rest) = current.strip_prefix("-") {
        current = rest.trim().to_string();
    } else if let Some(rest) = current.strip_prefix("*") {
        current = rest.trim().to_string();
    }

    let mut chars = current.chars().peekable();
    let mut digit_count = 0;
    while matches!(chars.peek(), Some(ch) if ch.is_ascii_digit()) {
        digit_count += 1;
        chars.next();
    }
    if digit_count > 0 && matches!(chars.peek(), Some('.')) {
        chars.next();
        current = chars.collect::<String>().trim().to_string();
    }

    current.trim().to_string()
}

fn collapse_whitespace(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn capitalize_first_ascii(message: &str) -> String {
    let mut chars = message.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
}

fn lowercase_first_ascii(message: &str) -> String {
    let mut chars = message.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    format!("{}{}", first.to_ascii_lowercase(), chars.as_str())
}

fn truncate_commit_subject(message: &str) -> String {
    if message.chars().count() <= COMMIT_SUBJECT_LIMIT {
        return message.to_string();
    }

    let mut truncated = String::new();
    let mut last_space_idx = None;
    for ch in message.chars() {
        let next_len = truncated.chars().count() + 1;
        if next_len > COMMIT_SUBJECT_LIMIT - 3 {
            break;
        }
        if ch.is_whitespace() {
            last_space_idx = Some(truncated.len());
        }
        truncated.push(ch);
    }

    if let Some(idx) = last_space_idx {
        truncated.truncate(idx);
    }

    format!("{}...", truncated.trim_end())
}

pub fn format_commit_message(message: &str) -> String {
    let first_line = message
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Update project");
    let mut subject = collapse_whitespace(&strip_task_prefixes(first_line))
        .trim_end_matches(['.', ':', ';'])
        .replace('`', "")
        .to_string();
    if subject.is_empty() {
        return "Update project".to_string();
    }

    let lower = subject.to_ascii_lowercase();
    let starts_with_action = COMMIT_ACTIONS
        .iter()
        .any(|action| lower == *action || lower.starts_with(&format!("{action} ")));
    if starts_with_action {
        for suffix in TEST_SUFFIXES {
            if let Some(idx) = lower.find(suffix) {
                subject.truncate(idx);
                subject = subject.trim_end().to_string();
                break;
            }
        }
        let lower = subject.to_ascii_lowercase();
        for suffix in CLAUSE_SUFFIXES {
            if let Some(idx) = lower.find(suffix) {
                subject.truncate(idx);
                subject = subject.trim_end().to_string();
                break;
            }
        }
    }

    let lower = subject.to_ascii_lowercase();
    let has_action = COMMIT_ACTIONS
        .iter()
        .any(|action| lower == *action || lower.starts_with(&format!("{action} ")));
    if has_action {
        truncate_commit_subject(&capitalize_first_ascii(&subject))
    } else {
        truncate_commit_subject(&format!("Implement {}", lowercase_first_ascii(&subject)))
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
    let result = git_output_with_retry(&["stash", "push", "-u", "-m", &msg])?;

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
    let list_result = git_output_with_retry(&["stash", "list"])?;

    let stash_list = String::from_utf8_lossy(&list_result.stdout);
    if stash_list.trim().is_empty() {
        return Ok(false);
    }

    // First, clean any current changes so the pop doesn't conflict
    let _ = git_output_with_retry(&["checkout", "--", "."]);
    let _ = git_output_with_retry(&["clean", "-fd"]);

    let pop_result = git_output_with_retry(&["stash", "pop"])?;

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
    let _ = git_output_with_retry(&["reset", "--hard", "HEAD"]);
    let _ = git_output_with_retry(&["clean", "-fd"]);

    // The failed pop leaves the stash in place — drop it
    let _ = git_output_with_retry(&["stash", "drop"]);

    Ok(true)
}

/// Drop the most recent stash entry (used after successful validation to discard the checkpoint).
/// Returns `Ok(true)` if a stash was dropped, `Ok(false)` if there is no stash.
pub fn checkpoint_drop() -> Result<bool> {
    if !git_is_repo() {
        return Err(DialError::NotGitRepo);
    }

    let list_result = git_output_with_retry(&["stash", "list"])?;

    let stash_list = String::from_utf8_lossy(&list_result.stdout);
    if stash_list.trim().is_empty() {
        return Ok(false);
    }

    let drop_result = git_output_with_retry(&["stash", "drop"])?;

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

const INTERNAL_EXCLUDE_PREFIXES: &[&str] = &[".dial", ".dial/", ".dial\\"];
const AGENT_INSTRUCTION_FILES: &[&str] = &["agents.md", "claude.md", "gemini.md"];

fn staged_files() -> Vec<String> {
    let output = git_output_with_retry(&["diff", "--cached", "--name-only"]);

    let Ok(result) = output else {
        return vec![];
    };

    String::from_utf8_lossy(&result.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn is_internal_excluded_path(file: &str) -> bool {
    let normalized = file.replace('\\', "/").to_ascii_lowercase();
    normalized == ".dial"
        || INTERNAL_EXCLUDE_PREFIXES
            .iter()
            .any(|prefix| normalized.starts_with(&prefix.replace('\\', "/")))
}

fn is_top_level_agent_instruction_path(file: &str) -> bool {
    let normalized = file
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_ascii_lowercase();
    !normalized.contains('/') && AGENT_INSTRUCTION_FILES.contains(&normalized.as_str())
}

fn path_exists_in_head(file: &str) -> bool {
    let normalized = file.replace('\\', "/");
    git_output_with_retry(&["cat-file", "-e", &format!("HEAD:{normalized}")])
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn auto_commit_excluded_files(staged: &[String]) -> Vec<String> {
    staged
        .iter()
        .filter(|file| {
            is_internal_excluded_path(file)
                || (is_top_level_agent_instruction_path(file) && !path_exists_in_head(file))
        })
        .cloned()
        .collect()
}

fn has_staged_changes() -> bool {
    git_output_with_retry(&["diff", "--cached", "--quiet", "--exit-code"])
        .map(|output| !output.status.success())
        .unwrap_or(false)
}

fn unstage_files(files: &[String]) {
    for file in files {
        let _ = git_output_with_retry(&["reset", "HEAD", "--", file]);
    }
}

/// Check staged files for potentially dangerous filenames.
/// Returns a list of warnings (empty if all safe).
fn check_staged_for_secrets(files: &[String]) -> Vec<String> {
    let mut warnings = Vec::new();

    for file in files {
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

fn git_commit_with_retry_policy(
    message: &str,
    retry_attempts: usize,
    retry_delay: Duration,
    stale_lock_age: Duration,
) -> Result<Option<String>> {
    ensure_git_commit_identity()?;

    // Stage all changes
    let add_result =
        git_output_with_retry_policy(&["add", "-A"], retry_attempts, retry_delay, stale_lock_age)?;

    if !add_result.status.success() {
        return Err(DialError::GitError(format!(
            "Failed to stage changes: {}",
            output_error_message(&add_result)
        )));
    }

    let staged = staged_files();

    // Safety check: warn about potentially dangerous files before committing
    let dangerous = check_staged_for_secrets(&staged);
    if !dangerous.is_empty() {
        eprintln!(
            "Warning: potentially sensitive files staged for commit: {}",
            dangerous.join(", ")
        );
        eprintln!("Add these to .gitignore if they should not be committed.");
        // Unstage the dangerous files and continue with the rest
        unstage_files(&dangerous);
    }

    let internal_files = auto_commit_excluded_files(&staged);
    if !internal_files.is_empty() {
        unstage_files(&internal_files);
    }

    if !has_staged_changes() {
        return Ok(None);
    }

    // Commit
    let formatted_message = format_commit_message(message);
    let commit_result = git_output_with_retry_policy(
        &["commit", "-m", &formatted_message],
        retry_attempts,
        retry_delay,
        stale_lock_age,
    )?;

    if !commit_result.status.success() {
        return Err(DialError::GitError(format!(
            "Failed to commit changes: {}",
            output_error_message(&commit_result)
        )));
    }

    // Get commit hash
    let hash_result = git_output_with_retry_policy(
        &["rev-parse", "HEAD"],
        retry_attempts,
        retry_delay,
        stale_lock_age,
    )?;

    if hash_result.status.success() {
        let hash = String::from_utf8_lossy(&hash_result.stdout)
            .trim()
            .to_string();
        Ok(Some(hash))
    } else {
        Ok(None)
    }
}

pub fn git_commit(message: &str) -> Result<Option<String>> {
    git_commit_with_retry_policy(
        message,
        INDEX_LOCK_RETRY_ATTEMPTS,
        Duration::from_millis(INDEX_LOCK_RETRY_DELAY_MS),
        Duration::from_secs(STALE_INDEX_LOCK_AGE_SECS),
    )
}

pub fn git_revert_to(commit_hash: &str) -> Result<bool> {
    let result = git_output_with_retry(&["reset", "--hard", commit_hash])?;

    Ok(result.status.success())
}

pub fn git_get_last_commit() -> Option<String> {
    git_output_with_retry(&["rev-parse", "HEAD"])
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Return the current working tree diff (unstaged changes) as a String.
pub fn git_diff() -> Result<String> {
    let result = git_output_with_retry(&["diff"])?;

    Ok(String::from_utf8_lossy(&result.stdout).to_string())
}

/// Return the current working tree diff stat (unstaged changes summary) as a String.
pub fn git_diff_stat() -> Result<String> {
    let result = git_output_with_retry(&["diff", "--stat"])?;

    Ok(String::from_utf8_lossy(&result.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    struct CwdGuard(PathBuf);

    impl CwdGuard {
        fn change_to(path: &std::path::Path) -> Self {
            let original = env::current_dir().unwrap();
            env::set_current_dir(path).unwrap();
            Self(original)
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.0);
        }
    }

    fn setup_git_repo() -> TempDir {
        let tmp = TempDir::new().unwrap();
        Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@dial.dev"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "DIAL Test"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        fs::write(tmp.path().join("README.md"), "initial\n").unwrap();
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial commit"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        tmp
    }

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

    #[test]
    fn test_internal_excluded_patterns_detect_dial_dir() {
        assert!(is_internal_excluded_path(".dial/default.db"));
        assert!(is_internal_excluded_path(".dial\\default.db-wal"));
        assert!(!is_internal_excluded_path("src/main.rs"));
    }

    #[test]
    fn test_agent_instruction_path_detection_is_top_level_only() {
        assert!(is_top_level_agent_instruction_path("AGENTS.md"));
        assert!(is_top_level_agent_instruction_path("./CLAUDE.md"));
        assert!(is_top_level_agent_instruction_path("GEMINI.md"));
        assert!(!is_top_level_agent_instruction_path("docs/AGENTS.md"));
        assert!(!is_top_level_agent_instruction_path("src/main.rs"));
    }

    #[test]
    #[serial(cwd)]
    fn test_git_commit_skips_dial_internal_files() {
        let tmp = setup_git_repo();
        let _guard = CwdGuard::change_to(tmp.path());

        fs::create_dir_all(".dial").unwrap();
        fs::write(".dial/default.db", "state").unwrap();
        fs::write("feature.txt", "real change\n").unwrap();

        let hash = git_commit("Add feature").unwrap().unwrap();
        assert!(!hash.is_empty());

        let show = Command::new("git")
            .args(["show", "--name-only", "--format=", "HEAD"])
            .output()
            .unwrap();
        let files = String::from_utf8_lossy(&show.stdout);
        assert!(files.contains("feature.txt"));
        assert!(!files.contains(".dial/default.db"));
    }

    #[test]
    #[serial(cwd)]
    fn test_git_commit_skips_new_agents_setup_file() {
        let tmp = setup_git_repo();
        let _guard = CwdGuard::change_to(tmp.path());

        fs::write("AGENTS.md", "local setup instructions\n").unwrap();
        fs::write("feature.txt", "real change\n").unwrap();

        let hash = git_commit("Add feature").unwrap().unwrap();
        assert!(!hash.is_empty());

        let show = Command::new("git")
            .args(["show", "--name-only", "--format=", "HEAD"])
            .output()
            .unwrap();
        let files = String::from_utf8_lossy(&show.stdout);
        assert!(files.contains("feature.txt"));
        assert!(!files.contains("AGENTS.md"));

        let status = Command::new("git")
            .args(["status", "--short"])
            .output()
            .unwrap();
        let status_output = String::from_utf8_lossy(&status.stdout);
        assert!(
            status_output.contains("?? AGENTS.md"),
            "expected AGENTS.md to remain untracked, got {status_output}"
        );
    }

    #[test]
    #[serial(cwd)]
    fn test_git_commit_keeps_tracked_agents_changes() {
        let tmp = setup_git_repo();
        let _guard = CwdGuard::change_to(tmp.path());

        fs::write("AGENTS.md", "shared instructions\n").unwrap();
        Command::new("git")
            .args(["add", "AGENTS.md"])
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "Add AGENTS"])
            .output()
            .unwrap();

        fs::write("AGENTS.md", "shared instructions\nupdated\n").unwrap();
        fs::write("feature.txt", "real change\n").unwrap();

        let hash = git_commit("Update feature").unwrap().unwrap();
        assert!(!hash.is_empty());

        let show = Command::new("git")
            .args(["show", "--name-only", "--format=", "HEAD"])
            .output()
            .unwrap();
        let files = String::from_utf8_lossy(&show.stdout);
        assert!(files.contains("feature.txt"));
        assert!(files.contains("AGENTS.md"));
    }

    #[test]
    #[serial(cwd)]
    fn test_git_commit_returns_none_when_only_dial_internal_files_changed() {
        let tmp = setup_git_repo();
        let _guard = CwdGuard::change_to(tmp.path());

        fs::create_dir_all(".dial").unwrap();
        fs::write(".dial/default.db", "state").unwrap();

        let hash = git_commit("Ignore internal state").unwrap();
        assert!(hash.is_none());
    }

    #[test]
    #[serial(cwd)]
    fn test_git_commit_retries_transient_index_lock() {
        let tmp = setup_git_repo();
        let _guard = CwdGuard::change_to(tmp.path());

        fs::write("feature.txt", "real change\n").unwrap();
        let lock_path = tmp.path().join(".git/index.lock");
        fs::write(&lock_path, "").unwrap();

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(60));
            let _ = fs::remove_file(lock_path);
        });

        let hash = git_commit_with_retry_policy(
            "Implement feature and add tests for it",
            6,
            Duration::from_millis(25),
            Duration::from_secs(60),
        )
        .unwrap()
        .unwrap();

        assert!(!hash.is_empty());
    }

    #[test]
    #[serial(cwd)]
    fn test_git_commit_clears_stale_index_lock() {
        let tmp = setup_git_repo();
        let _guard = CwdGuard::change_to(tmp.path());

        fs::write("feature.txt", "real change\n").unwrap();
        let lock_path = tmp.path().join(".git/index.lock");
        fs::write(&lock_path, "").unwrap();

        let hash = git_commit_with_retry_policy(
            "Implement feature and add tests for it",
            2,
            Duration::from_millis(10),
            Duration::ZERO,
        )
        .unwrap()
        .unwrap();

        assert!(!hash.is_empty());
        assert!(!lock_path.exists());
    }

    #[test]
    #[serial(cwd)]
    fn test_git_commit_errors_on_persistent_index_lock() {
        let tmp = setup_git_repo();
        let _guard = CwdGuard::change_to(tmp.path());

        fs::write("feature.txt", "real change\n").unwrap();
        fs::write(tmp.path().join(".git/index.lock"), "").unwrap();

        let error = git_commit_with_retry_policy(
            "Implement feature and add tests for it",
            2,
            Duration::from_millis(10),
            Duration::from_secs(60),
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("index.lock"),
            "expected index.lock error, got {error}"
        );
    }

    #[test]
    fn test_format_commit_message_normalizes_task_description() {
        let message = format_commit_message(
            "implement status-based checkbox formatting in src/noteFormatter.js and add tests for todo, done, and missing status rendering.",
        );
        assert_eq!(
            message,
            "Implement status-based checkbox formatting in src/noteFormatter.js"
        );
    }

    #[test]
    fn test_format_commit_message_prefixes_missing_action() {
        let message = format_commit_message("status-based checkbox formatting for notes");
        assert_eq!(
            message,
            "Implement status-based checkbox formatting for notes"
        );
    }

    #[test]
    fn test_format_commit_message_keeps_existing_finish_action() {
        let message = format_commit_message(
            "Finish `src/cli.js` and extend `test/noteFormatter.test.js` so the CLI reads stdin and fails cleanly on invalid JSON.",
        );
        assert_eq!(
            message,
            "Finish src/cli.js and extend test/noteFormatter.test.js"
        );
    }

    #[test]
    fn test_format_commit_message_trims_so_clause_for_update_subject() {
        let message = format_commit_message(
            "Update `src/noteFormatter.js` and `test/noteFormatter.test.js` so note tags are lowercased, trimmed, and deduplicated.",
        );
        assert_eq!(
            message,
            "Update src/noteFormatter.js and test/noteFormatter.test.js"
        );
    }

    #[test]
    #[serial(cwd)]
    fn test_git_commit_restores_missing_identity_from_head_author() {
        let tmp = setup_git_repo();
        let _guard = CwdGuard::change_to(tmp.path());
        let no_global = tmp.path().join("no-global.gitconfig");
        fs::write(&no_global, "").unwrap();
        let previous_global = env::var_os("GIT_CONFIG_GLOBAL");
        env::set_var("GIT_CONFIG_GLOBAL", &no_global);

        Command::new("git")
            .args(["config", "--unset", "user.name"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "--unset", "user.email"])
            .current_dir(tmp.path())
            .output()
            .unwrap();

        fs::write("feature.txt", "real change\n").unwrap();
        let hash = git_commit("Add feature").unwrap().unwrap();
        assert!(!hash.is_empty());

        let restored_name = Command::new("git")
            .args(["config", "--get", "user.name"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        let restored_email = Command::new("git")
            .args(["config", "--get", "user.email"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&restored_name.stdout).trim(),
            "DIAL Test"
        );
        assert_eq!(
            String::from_utf8_lossy(&restored_email.stdout).trim(),
            "test@dial.dev"
        );

        if let Some(previous) = previous_global {
            env::set_var("GIT_CONFIG_GLOBAL", previous);
        } else {
            env::remove_var("GIT_CONFIG_GLOBAL");
        }
    }
}
