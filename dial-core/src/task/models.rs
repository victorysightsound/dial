use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Cancelled,
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "pending"),
            TaskStatus::InProgress => write!(f, "in_progress"),
            TaskStatus::Completed => write!(f, "completed"),
            TaskStatus::Blocked => write!(f, "blocked"),
            TaskStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl TaskStatus {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(TaskStatus::Pending),
            "in_progress" => Some(TaskStatus::InProgress),
            "completed" => Some(TaskStatus::Completed),
            "blocked" => Some(TaskStatus::Blocked),
            "cancelled" => Some(TaskStatus::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub description: String,
    pub status: TaskStatus,
    pub priority: i32,
    pub blocked_by: Option<String>,
    pub spec_section_id: Option<i64>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl Task {
    pub fn from_row(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        let status_str: String = row.get("status")?;
        Ok(Task {
            id: row.get("id")?,
            description: row.get("description")?,
            status: TaskStatus::from_str(&status_str).unwrap_or(TaskStatus::Pending),
            priority: row.get("priority")?,
            blocked_by: row.get("blocked_by")?,
            spec_section_id: row.get("spec_section_id")?,
            created_at: row.get("created_at")?,
            started_at: row.get("started_at")?,
            completed_at: row.get("completed_at")?,
        })
    }
}
