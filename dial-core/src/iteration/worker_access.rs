use crate::db::with_transaction;
use crate::errors::{DialError, Result};
use crate::task::models::Task;
use chrono::Local;
use rusqlite::Connection;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

pub const READ_ONLY_WORKER_REASON: &str =
    "worker workspace is read-only; implementation tasks require write access";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerAccess {
    Writable,
    ReadOnly { detail: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerAccessHint {
    Writable,
    ReadOnly,
    Unknown,
}

pub fn probe_worker_write_access(workdir: &Path) -> Result<WorkerAccess> {
    let probe_dir = workdir.join(".dial").join("worker-access");
    if let Err(err) = fs::create_dir_all(&probe_dir) {
        return Ok(WorkerAccess::ReadOnly {
            detail: format!("unable to prepare {}: {}", probe_dir.display(), err),
        });
    }

    let probe_name = format!("worker-write-probe-{}.tmp", unique_probe_suffix());
    let probe_path = probe_dir.join(probe_name);
    let payload = "dial worker write probe";

    match write_read_delete_probe(&probe_path, payload) {
        Ok(()) => Ok(WorkerAccess::Writable),
        Err(detail) => Ok(WorkerAccess::ReadOnly { detail }),
    }
}

pub fn record_worker_access_block(
    conn: &Connection,
    task: &Task,
    backend_name: &str,
    explicit_read_only: bool,
    probe_detail: &str,
) -> Result<()> {
    let probe_result = if explicit_read_only {
        "explicit_read_only"
    } else {
        "probe_read_only"
    };
    let probe_detail = probe_detail.trim();
    let blocked_reason = READ_ONLY_WORKER_REASON.to_string();

    with_transaction(conn, |conn| {
        let changed = conn.execute(
            "UPDATE tasks SET status = 'blocked', blocked_by = ?1 WHERE id = ?2",
            rusqlite::params![blocked_reason, task.id],
        )?;

        if changed == 0 {
            return Err(DialError::TaskNotFound(task.id));
        }

        conn.execute(
            "INSERT INTO worker_access_checks (
                task_id, backend_name, probe_result, blocked_reason, probe_detail, explicitly_read_only
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                task.id,
                backend_name,
                probe_result,
                READ_ONLY_WORKER_REASON,
                if probe_detail.is_empty() {
                    None::<String>
                } else {
                    Some(probe_detail.to_string())
                },
                if explicit_read_only { 1 } else { 0 },
            ],
        )?;

        Ok(())
    })?;

    let _ = crate::artifacts::sync_task_ledger(conn);
    Ok(())
}

fn write_read_delete_probe(path: &Path, payload: &str) -> std::result::Result<(), String> {
    let mut file = fs::File::create(path)
        .map_err(|err| format!("unable to create probe file {}: {}", path.display(), err))?;

    file.write_all(payload.as_bytes())
        .and_then(|_| file.flush())
        .map_err(|err| format!("unable to write probe file {}: {}", path.display(), err))?;

    let observed = fs::read_to_string(path)
        .map_err(|err| format!("unable to read probe file {}: {}", path.display(), err))?;
    if observed != payload {
        let _ = fs::remove_file(path);
        return Err(format!(
            "probe file {} did not round-trip expected content",
            path.display()
        ));
    }

    fs::remove_file(path)
        .map_err(|err| format!("unable to delete probe file {}: {}", path.display(), err))?;

    Ok(())
}

fn unique_probe_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{}-{}-{}", process::id(), Local::now().timestamp_millis(), nanos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;
    use crate::db::schema;
    use crate::task::models::{Task, TaskStatus};
    use tempfile::tempdir;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .unwrap();
        conn.execute_batch(schema::SCHEMA).unwrap();
        migrations::run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO tasks (description, priority) VALUES (?1, ?2)",
            rusqlite::params!["Test task", 5],
        )
        .unwrap();
        conn
    }

    fn make_task() -> Task {
        Task {
            id: 1,
            description: "Test task".to_string(),
            status: TaskStatus::Pending,
            priority: 5,
            blocked_by: None,
            spec_section_id: None,
            prd_section_id: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            started_at: None,
            completed_at: None,
            total_attempts: 0,
            total_failures: 0,
            last_failure_at: None,
            acceptance_criteria: vec![],
            requires_browser_verification: false,
        }
    }

    #[test]
    fn probe_worker_write_access_succeeds_for_writable_workspace() {
        let tmp = tempdir().unwrap();
        let result = probe_worker_write_access(tmp.path()).unwrap();
        assert_eq!(result, WorkerAccess::Writable);
        assert!(tmp.path().join(".dial/worker-access").is_dir());
    }

    #[test]
    fn probe_worker_write_access_fails_for_read_only_workspace() {
        let tmp = tempdir().unwrap();
        let probe_dir = tmp.path().join(".dial/worker-access");
        fs::create_dir_all(&probe_dir).unwrap();
        let mut perms = fs::metadata(&probe_dir).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&probe_dir, perms).unwrap();

        let result = probe_worker_write_access(tmp.path()).unwrap();
        match result {
            WorkerAccess::Writable => panic!("expected read-only probe result"),
            WorkerAccess::ReadOnly { detail } => {
                assert!(!detail.trim().is_empty());
            }
        }
    }

    #[test]
    fn record_worker_access_block_marks_task_blocked_and_records_audit() {
        let conn = setup_test_db();
        let task = make_task();
        let result = record_worker_access_block(
            &conn,
            &task,
            "Codex CLI",
            false,
            "probe failed: permission denied",
        );

        assert!(result.is_ok());

        let status: (String, String) = conn
            .query_row(
                "SELECT status, blocked_by FROM tasks WHERE id = ?1",
                [task.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status.0, "blocked");
        assert_eq!(status.1, READ_ONLY_WORKER_REASON);

        let row: (i64, String, String, String, Option<String>, i64) = conn
            .query_row(
                "SELECT task_id, backend_name, probe_result, blocked_reason, probe_detail, explicitly_read_only
                 FROM worker_access_checks WHERE task_id = ?1",
                [task.id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, task.id);
        assert_eq!(row.1, "Codex CLI");
        assert_eq!(row.2, "probe_read_only");
        assert_eq!(row.3, READ_ONLY_WORKER_REASON);
        assert_eq!(
            row.4.as_deref(),
            Some("probe failed: permission denied")
        );
        assert_eq!(row.5, 0);
    }
}
