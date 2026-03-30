use crate::db::get_dial_dir;
use crate::errors::{DialError, Result};
use serde::{Deserialize, Serialize};
use serde_json::error::Category as JsonErrorCategory;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use super::orchestrator::SubagentResult;

/// Structured signal emitted by a subagent via `.dial/signal.json`.
///
/// This replaces the regex-parsed `DIAL_COMPLETE` / `DIAL_BLOCKED` / `DIAL_LEARNING`
/// text markers with a machine-readable JSON file, eliminating false positives from
/// template placeholders and markdown formatting variations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubagentSignal {
    Complete {
        summary: String,
    },
    Blocked {
        reason: String,
    },
    Learning {
        category: String,
        description: String,
    },
}

/// JSON envelope written by the subagent to `.dial/signal.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignalFile {
    pub signals: Vec<SubagentSignal>,
    pub timestamp: String,
}

const SIGNAL_READ_ATTEMPTS: usize = 5;
const SIGNAL_READ_RETRY_DELAY_MS: u64 = 50;

/// Default path for the signal file: `.dial/signal.json`
pub fn signal_file_path() -> PathBuf {
    get_dial_dir().join("signal.json")
}

/// Signal file path relative to a given base directory.
pub fn signal_file_path_at(base: &Path) -> PathBuf {
    base.join(".dial").join("signal.json")
}

/// Read and parse `.dial/signal.json`, then delete it.
///
/// Returns `Ok(Some(file))` if the file exists and parses successfully,
/// `Ok(None)` if the file does not exist, and `Err` on parse/IO errors.
pub fn read_signal_file() -> Result<Option<SignalFile>> {
    read_signal_file_at(&signal_file_path())
}

/// Read and parse a signal file at the given path, then delete it.
pub fn read_signal_file_at(path: &Path) -> Result<Option<SignalFile>> {
    let mut last_retryable_error = None;

    for attempt in 0..SIGNAL_READ_ATTEMPTS {
        if !path.exists() {
            return Ok(None);
        }

        match try_read_signal_file_once(path)? {
            SignalReadOutcome::Parsed(signal_file) => return Ok(Some(signal_file)),
            SignalReadOutcome::Retryable(error) => {
                last_retryable_error = Some(error);

                if attempt + 1 < SIGNAL_READ_ATTEMPTS {
                    thread::sleep(Duration::from_millis(SIGNAL_READ_RETRY_DELAY_MS));
                    continue;
                }
            }
        }
    }

    Err(last_retryable_error.unwrap_or_else(|| {
        DialError::CommandFailed("Failed to read signal file after retries".to_string())
    }))
}

/// Write a signal file to `.dial/signal.json` (primarily for testing).
pub fn write_signal_file(signal_file: &SignalFile) -> Result<()> {
    write_signal_file_at(&signal_file_path(), signal_file)
}

/// Write a signal file to the given path.
pub fn write_signal_file_at(path: &Path, signal_file: &SignalFile) -> Result<()> {
    let json = serde_json::to_string_pretty(signal_file)
        .map_err(|e| DialError::CommandFailed(format!("Failed to serialize signal file: {}", e)))?;

    fs::write(path, json)
        .map_err(|e| DialError::CommandFailed(format!("Failed to write signal file: {}", e)))?;

    Ok(())
}

enum SignalReadOutcome {
    Parsed(SignalFile),
    Retryable(DialError),
}

fn try_read_signal_file_once(path: &Path) -> Result<SignalReadOutcome> {
    let bytes = fs::read(path)
        .map_err(|e| DialError::CommandFailed(format!("Failed to read signal file: {}", e)))?;

    if bytes.is_empty() {
        return Ok(SignalReadOutcome::Retryable(DialError::CommandFailed(
            "Signal file is empty".to_string(),
        )));
    }

    let contents = decode_signal_file_bytes(&bytes)?;
    if contents.trim().is_empty() {
        return Ok(SignalReadOutcome::Retryable(DialError::CommandFailed(
            "Signal file is empty".to_string(),
        )));
    }

    let signal_file: SignalFile = match serde_json::from_str(&contents) {
        Ok(signal_file) => signal_file,
        Err(e) if signal_parse_error_is_retryable(&e) => {
            return Ok(SignalReadOutcome::Retryable(DialError::CommandFailed(
                format!("Failed to parse signal file: {}", e),
            )));
        }
        Err(e) => {
            return Err(DialError::CommandFailed(format!(
                "Failed to parse signal file: {}",
                e
            )));
        }
    };

    // Delete the file after successful parse so it's not re-read
    fs::remove_file(path)
        .map_err(|e| DialError::CommandFailed(format!("Failed to delete signal file: {}", e)))?;

    Ok(SignalReadOutcome::Parsed(signal_file))
}

fn decode_signal_file_bytes(bytes: &[u8]) -> Result<String> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(bytes[3..].to_vec()).map_err(|e| {
            DialError::CommandFailed(format!("Failed to decode signal file as UTF-8: {}", e))
        });
    }

    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16_signal_bytes(&bytes[2..], true);
    }

    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16_signal_bytes(&bytes[2..], false);
    }

    String::from_utf8(bytes.to_vec()).map_err(|e| {
        DialError::CommandFailed(format!("Failed to decode signal file as UTF-8: {}", e))
    })
}

fn decode_utf16_signal_bytes(bytes: &[u8], little_endian: bool) -> Result<String> {
    if bytes.len() % 2 != 0 {
        return Err(DialError::CommandFailed(
            "Failed to decode signal file as UTF-16: odd-length byte stream".to_string(),
        ));
    }

    let code_units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect();

    String::from_utf16(&code_units).map_err(|e| {
        DialError::CommandFailed(format!("Failed to decode signal file as UTF-16: {}", e))
    })
}

fn signal_parse_error_is_retryable(error: &serde_json::Error) -> bool {
    matches!(error.classify(), JsonErrorCategory::Eof) || (error.line() == 1 && error.column() == 1)
}

/// Convert a `SignalFile` into a `SubagentResult` for compatibility with
/// the existing orchestrator flow.
pub fn signal_file_to_result(signal_file: &SignalFile, raw_output: &str) -> SubagentResult {
    let mut result = SubagentResult {
        raw_output: raw_output.to_string(),
        ..Default::default()
    };

    for signal in &signal_file.signals {
        match signal {
            SubagentSignal::Complete { summary } => {
                result.complete = true;
                result.complete_message = Some(summary.clone());
            }
            SubagentSignal::Blocked { reason } => {
                result.blocked = true;
                result.blocked_message = Some(reason.clone());
            }
            SubagentSignal::Learning {
                category,
                description,
            } => {
                result
                    .learnings
                    .push((category.clone(), description.clone()));
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: set up a temp dir with `.dial/` subdirectory, return TempDir and signal path.
    /// Uses path-based functions — no CWD change needed.
    fn setup_temp_signal_path() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dial_dir = tmp.path().join(".dial");
        fs::create_dir_all(&dial_dir).unwrap();
        let path = signal_file_path_at(tmp.path());
        (tmp, path)
    }

    #[test]
    fn test_parse_signal_file_complete() {
        let json = r#"{
            "signals": [
                {"type": "complete", "summary": "Implemented feature X"}
            ],
            "timestamp": "2026-03-12T10:00:00Z"
        }"#;

        let signal_file: SignalFile = serde_json::from_str(json).unwrap();
        assert_eq!(signal_file.signals.len(), 1);
        assert_eq!(
            signal_file.signals[0],
            SubagentSignal::Complete {
                summary: "Implemented feature X".to_string()
            }
        );
        assert_eq!(signal_file.timestamp, "2026-03-12T10:00:00Z");
    }

    #[test]
    fn test_parse_signal_file_blocked() {
        let json = r#"{
            "signals": [
                {"type": "blocked", "reason": "Missing API credentials"}
            ],
            "timestamp": "2026-03-12T10:00:00Z"
        }"#;

        let signal_file: SignalFile = serde_json::from_str(json).unwrap();
        assert_eq!(signal_file.signals.len(), 1);
        assert_eq!(
            signal_file.signals[0],
            SubagentSignal::Blocked {
                reason: "Missing API credentials".to_string()
            }
        );
    }

    #[test]
    fn test_parse_signal_file_learning() {
        let json = r#"{
            "signals": [
                {"type": "learning", "category": "pattern", "description": "Use Option<T> for nullable fields"}
            ],
            "timestamp": "2026-03-12T10:00:00Z"
        }"#;

        let signal_file: SignalFile = serde_json::from_str(json).unwrap();
        assert_eq!(signal_file.signals.len(), 1);
        assert_eq!(
            signal_file.signals[0],
            SubagentSignal::Learning {
                category: "pattern".to_string(),
                description: "Use Option<T> for nullable fields".to_string()
            }
        );
    }

    #[test]
    fn test_parse_multiple_signals() {
        let json = r#"{
            "signals": [
                {"type": "learning", "category": "gotcha", "description": "Watch out for null pointers"},
                {"type": "learning", "category": "pattern", "description": "Use Option<T>"},
                {"type": "complete", "summary": "Implemented the feature"}
            ],
            "timestamp": "2026-03-12T10:00:00Z"
        }"#;

        let signal_file: SignalFile = serde_json::from_str(json).unwrap();
        assert_eq!(signal_file.signals.len(), 3);
        assert!(matches!(
            &signal_file.signals[0],
            SubagentSignal::Learning { .. }
        ));
        assert!(matches!(
            &signal_file.signals[1],
            SubagentSignal::Learning { .. }
        ));
        assert!(matches!(
            &signal_file.signals[2],
            SubagentSignal::Complete { .. }
        ));
    }

    #[test]
    fn test_signal_file_to_result_complete() {
        let signal_file = SignalFile {
            signals: vec![SubagentSignal::Complete {
                summary: "Done with task".to_string(),
            }],
            timestamp: "2026-03-12T10:00:00Z".to_string(),
        };

        let result = signal_file_to_result(&signal_file, "raw output here");
        assert!(result.complete);
        assert_eq!(result.complete_message, Some("Done with task".to_string()));
        assert!(!result.blocked);
        assert!(result.learnings.is_empty());
        assert_eq!(result.raw_output, "raw output here");
    }

    #[test]
    fn test_signal_file_to_result_blocked() {
        let signal_file = SignalFile {
            signals: vec![SubagentSignal::Blocked {
                reason: "Need credentials".to_string(),
            }],
            timestamp: "2026-03-12T10:00:00Z".to_string(),
        };

        let result = signal_file_to_result(&signal_file, "");
        assert!(result.blocked);
        assert_eq!(result.blocked_message, Some("Need credentials".to_string()));
        assert!(!result.complete);
    }

    #[test]
    fn test_signal_file_to_result_mixed() {
        let signal_file = SignalFile {
            signals: vec![
                SubagentSignal::Learning {
                    category: "pattern".to_string(),
                    description: "Use parameterized SQL".to_string(),
                },
                SubagentSignal::Learning {
                    category: "gotcha".to_string(),
                    description: "Watch for race conditions".to_string(),
                },
                SubagentSignal::Complete {
                    summary: "Feature implemented".to_string(),
                },
            ],
            timestamp: "2026-03-12T10:00:00Z".to_string(),
        };

        let result = signal_file_to_result(&signal_file, "output");
        assert!(result.complete);
        assert_eq!(
            result.complete_message,
            Some("Feature implemented".to_string())
        );
        assert_eq!(result.learnings.len(), 2);
        assert_eq!(
            result.learnings[0],
            ("pattern".to_string(), "Use parameterized SQL".to_string())
        );
        assert_eq!(
            result.learnings[1],
            (
                "gotcha".to_string(),
                "Watch for race conditions".to_string()
            )
        );
    }

    #[test]
    fn test_signal_file_to_result_empty() {
        let signal_file = SignalFile {
            signals: vec![],
            timestamp: "2026-03-12T10:00:00Z".to_string(),
        };

        let result = signal_file_to_result(&signal_file, "raw");
        assert!(!result.complete);
        assert!(!result.blocked);
        assert!(result.learnings.is_empty());
    }

    #[test]
    fn test_write_and_read_signal_file() {
        let (_tmp, path) = setup_temp_signal_path();

        let signal_file = SignalFile {
            signals: vec![
                SubagentSignal::Learning {
                    category: "pattern".to_string(),
                    description: "Always validate inputs".to_string(),
                },
                SubagentSignal::Complete {
                    summary: "Task done".to_string(),
                },
            ],
            timestamp: "2026-03-12T10:00:00Z".to_string(),
        };

        // Write
        write_signal_file_at(&path, &signal_file).unwrap();
        assert!(path.exists());

        // Read (should parse and delete)
        let read_back = read_signal_file_at(&path).unwrap();
        assert!(read_back.is_some());
        let read_back = read_back.unwrap();
        assert_eq!(read_back, signal_file);

        // File should be deleted after read
        assert!(!path.exists());
    }

    #[test]
    fn test_read_signal_file_missing() {
        let (_tmp, path) = setup_temp_signal_path();

        // Remove the file to ensure it doesn't exist
        let _ = fs::remove_file(&path);

        let result = read_signal_file_at(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_read_signal_file_invalid_json() {
        let (_tmp, path) = setup_temp_signal_path();

        fs::write(&path, "not valid json").unwrap();

        let result = read_signal_file_at(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_signal_file_with_utf8_bom() {
        let (_tmp, path) = setup_temp_signal_path();

        let json = concat!(
            "\u{feff}",
            "{",
            "\"signals\":[{\"type\":\"complete\",\"summary\":\"Done\"}],",
            "\"timestamp\":\"2026-03-29T12:00:00Z\"",
            "}"
        );

        fs::write(&path, json.as_bytes()).unwrap();

        let result = read_signal_file_at(&path).unwrap().unwrap();
        assert_eq!(
            result,
            SignalFile {
                signals: vec![SubagentSignal::Complete {
                    summary: "Done".to_string(),
                }],
                timestamp: "2026-03-29T12:00:00Z".to_string(),
            }
        );
    }

    #[test]
    fn test_read_signal_file_with_utf16le_bom() {
        let (_tmp, path) = setup_temp_signal_path();

        let json = "{\"signals\":[{\"type\":\"complete\",\"summary\":\"Done\"}],\"timestamp\":\"2026-03-29T12:00:00Z\"}";
        let mut bytes = vec![0xFF, 0xFE];
        for code_unit in json.encode_utf16() {
            bytes.extend_from_slice(&code_unit.to_le_bytes());
        }
        fs::write(&path, bytes).unwrap();

        let result = read_signal_file_at(&path).unwrap().unwrap();
        assert_eq!(
            result,
            SignalFile {
                signals: vec![SubagentSignal::Complete {
                    summary: "Done".to_string(),
                }],
                timestamp: "2026-03-29T12:00:00Z".to_string(),
            }
        );
    }

    #[test]
    fn test_read_signal_file_retries_after_empty_placeholder() {
        let (_tmp, path) = setup_temp_signal_path();

        let expected = SignalFile {
            signals: vec![SubagentSignal::Complete {
                summary: "Recovered after retry".to_string(),
            }],
            timestamp: "2026-03-29T12:30:00Z".to_string(),
        };
        let json = serde_json::to_string(&expected).unwrap();

        fs::write(&path, "").unwrap();

        let path_for_writer = path.clone();
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(SIGNAL_READ_RETRY_DELAY_MS * 2));
            fs::write(&path_for_writer, json).unwrap();
        });

        let result = read_signal_file_at(&path).unwrap().unwrap();
        writer.join().unwrap();

        assert_eq!(result, expected);
        assert!(!path.exists());
    }

    #[test]
    fn test_serialize_signal_file_roundtrip() {
        let signal_file = SignalFile {
            signals: vec![
                SubagentSignal::Complete {
                    summary: "Done".to_string(),
                },
                SubagentSignal::Blocked {
                    reason: "Stuck".to_string(),
                },
                SubagentSignal::Learning {
                    category: "test".to_string(),
                    description: "Roundtrip works".to_string(),
                },
            ],
            timestamp: "2026-03-12T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&signal_file).unwrap();
        let deserialized: SignalFile = serde_json::from_str(&json).unwrap();
        assert_eq!(signal_file, deserialized);
    }
}
