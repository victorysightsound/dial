use dial_core::prd;
use dial_core::Engine;
use serde_json::json;
use std::env;
use std::sync::Mutex;
use tempfile::TempDir;

static CWD_LOCK: Mutex<()> = Mutex::new(());

/// Lock that recovers from poison (prior test panics).
fn lock() -> std::sync::MutexGuard<'static, ()> {
    CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Helper: create an Engine in a temp directory.
async fn setup_engine() -> (Engine, TempDir, std::path::PathBuf) {
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let engine = Engine::init("test", None, false).await.unwrap();
    (engine, tmp, original_dir)
}

// --- Import Pipeline Tests ---

#[tokio::test]
async fn test_prd_import_from_markdown_directory() {
    let _lock = lock();
    let (_engine, tmp, original_dir) = setup_engine().await;

    let specs_dir = tmp.path().join("specs");
    std::fs::create_dir_all(&specs_dir).unwrap();

    std::fs::write(
        specs_dir.join("overview.md"),
        "# Overview\n\nThis is the project overview.\n\n## Goals\n\nBuild something great.\n\n## Non-Goals\n\nNot building something mediocre.\n",
    ).unwrap();

    std::fs::write(
        specs_dir.join("architecture.md"),
        "# Architecture\n\nThe system uses a modular design.\n\n## Components\n\nThere are three main components.\n",
    ).unwrap();

    let result = prd::import::prd_import("specs").unwrap();
    assert_eq!(result.files, 2);
    assert!(result.sections >= 5);

    assert!(prd::prd_db_exists());

    let conn = prd::get_prd_db().unwrap();
    let sections = prd::prd_list_sections(&conn).unwrap();
    assert!(sections.len() >= 5);

    // h2 sections should have parent_ids
    let h2_sections: Vec<_> = sections.iter().filter(|s| s.level == 2).collect();
    assert!(!h2_sections.is_empty());
    for s in &h2_sections {
        assert!(s.parent_id.is_some(), "h2 section '{}' should have a parent_id", s.title);
    }

    // FTS search
    let results = prd::prd_search_sections(&conn, "modular").unwrap();
    assert!(!results.is_empty(), "FTS search should find 'modular'");

    // Sources recorded
    let sources = prd::prd_list_sources(&conn).unwrap();
    assert_eq!(sources.len(), 2);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_prd_import_single_file() {
    let _lock = lock();
    let (_engine, tmp, original_dir) = setup_engine().await;

    let md_file = tmp.path().join("test.md");
    std::fs::write(
        &md_file,
        "# Test Doc\n\nSome content.\n\n## Section A\n\nDetails about A.\n",
    ).unwrap();

    let count = prd::import::prd_import_file(&md_file).unwrap();
    assert_eq!(count, 2);

    let conn = prd::get_prd_db().unwrap();
    let sections = prd::prd_list_sections(&conn).unwrap();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].title, "Test Doc");
    assert_eq!(sections[1].title, "Section A");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_prd_import_empty_directory() {
    let _lock = lock();
    let (_engine, tmp, original_dir) = setup_engine().await;

    let specs_dir = tmp.path().join("empty_specs");
    std::fs::create_dir_all(&specs_dir).unwrap();

    let result = prd::import::prd_import("empty_specs").unwrap();
    assert_eq!(result.files, 0);
    assert_eq!(result.sections, 0);

    env::set_current_dir(original_dir).unwrap();
}

// --- Terminology Tests ---

#[tokio::test]
async fn test_prd_terminology_crud() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let conn = prd::get_or_init_prd_db().unwrap();

    let id = prd::prd_add_term(&conn, "API", "[\"api\", \"Rest API\"]", "Application Programming Interface", "technical", None).unwrap();
    assert!(id > 0);

    prd::prd_add_term(&conn, "PRD", "[\"prd\"]", "Product Requirements Document", "domain", Some("overview")).unwrap();
    prd::prd_add_term(&conn, "FTS", "[]", "Full-Text Search", "technical", None).unwrap();

    let all = prd::prd_list_terms(&conn, None).unwrap();
    assert_eq!(all.len(), 3);

    let tech = prd::prd_list_terms(&conn, Some("technical")).unwrap();
    assert_eq!(tech.len(), 2);

    let results = prd::prd_search_terms(&conn, "Application").unwrap();
    assert!(!results.is_empty());
    assert_eq!(results[0].canonical, "API");

    prd::prd_delete_term(&conn, "API").unwrap();
    let remaining = prd::prd_list_terms(&conn, None).unwrap();
    assert_eq!(remaining.len(), 2);

    env::set_current_dir(original_dir).unwrap();
}

// --- Section CRUD Tests ---

#[tokio::test]
async fn test_prd_section_crud() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let conn = prd::get_or_init_prd_db().unwrap();

    prd::prd_insert_section(&conn, "1", "Overview", None, 1, 0, "This is the overview.", 4).unwrap();
    prd::prd_insert_section(&conn, "1.1", "Goals", Some("1"), 2, 1, "Our goals are clear.", 4).unwrap();
    prd::prd_insert_section(&conn, "1.2", "Non-Goals", Some("1"), 2, 2, "What we won't do.", 4).unwrap();

    let section = prd::prd_get_section(&conn, "1.1").unwrap();
    assert!(section.is_some());
    let section = section.unwrap();
    assert_eq!(section.title, "Goals");
    assert_eq!(section.parent_id.as_deref(), Some("1"));

    let all = prd::prd_list_sections(&conn).unwrap();
    assert_eq!(all.len(), 3);

    prd::prd_update_section(&conn, "1.1", "Our goals are very clear and well-defined.").unwrap();
    let updated = prd::prd_get_section(&conn, "1.1").unwrap().unwrap();
    assert!(updated.content.contains("well-defined"));
    assert_eq!(updated.word_count, 7);

    let results = prd::prd_search_sections(&conn, "goals").unwrap();
    assert!(!results.is_empty());

    env::set_current_dir(original_dir).unwrap();
}

// --- Context Assembly with PRD ---

#[tokio::test]
async fn test_context_assembly_uses_prd_when_available() {
    let _lock = lock();
    let (engine, _tmp, original_dir) = setup_engine().await;

    // Create prd.db with sections containing distinctive content
    let conn = prd::get_or_init_prd_db().unwrap();
    prd::prd_insert_section(&conn, "1", "Authentication", None, 1, 0, "Users authenticate via OAuth2 tokens and bearer authentication.", 7).unwrap();
    prd::prd_insert_section(&conn, "2", "Data Model", None, 1, 1, "The data model uses SQLite with FTS5 for search.", 9).unwrap();
    drop(conn);

    // Add a task whose description matches PRD content
    let task_id = engine.task_add("Implement authentication", 1, None).await.unwrap();
    let task = engine.task_get(task_id).await.unwrap();

    // Verify prd.db is detected
    assert!(prd::prd_db_exists());

    // Context gathering should work without error
    let phase_conn = dial_core::get_db(Some("test")).unwrap();
    let context = dial_core::iteration::context::gather_context(&phase_conn, &task).unwrap();

    // Context should at minimum contain signs
    assert!(context.contains("SIGNS") || context.contains("ONE TASK ONLY"));

    env::set_current_dir(original_dir).unwrap();
}

// --- Backward Compatibility ---

#[tokio::test]
async fn test_context_falls_back_to_spec_sections_without_prd() {
    let _lock = lock();
    let (engine, _tmp, original_dir) = setup_engine().await;

    assert!(!prd::prd_db_exists());

    let task_id = engine.task_add("Build the widget", 1, None).await.unwrap();
    let task = engine.task_get(task_id).await.unwrap();

    let phase_conn = dial_core::get_db(Some("test")).unwrap();
    let context = dial_core::iteration::context::gather_context(&phase_conn, &task).unwrap();
    assert!(!context.is_empty(), "Context should still include signs and other non-spec content");

    env::set_current_dir(original_dir).unwrap();
}

// --- Wizard State Persistence ---

#[tokio::test]
async fn test_wizard_state_save_and_load() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let conn = prd::get_or_init_prd_db().unwrap();

    let mut state = prd::wizard::WizardState::new("mvp");
    assert_eq!(state.current_phase, prd::wizard::WizardPhase::Vision);
    assert!(state.completed_phases.is_empty());

    prd::wizard::save_wizard_state(&conn, &state).unwrap();

    let loaded = prd::wizard::load_wizard_state(&conn).unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.template, "mvp");
    assert_eq!(loaded.current_phase, prd::wizard::WizardPhase::Vision);

    state.mark_phase_complete(prd::wizard::WizardPhase::Vision);
    state.set_phase_data(prd::wizard::WizardPhase::Vision, serde_json::json!({
        "problem": "Users need better specs",
        "target_users": "Developers"
    }));
    prd::wizard::save_wizard_state(&conn, &state).unwrap();

    let reloaded = prd::wizard::load_wizard_state(&conn).unwrap().unwrap();
    assert_eq!(reloaded.current_phase, prd::wizard::WizardPhase::Functionality);
    assert!(reloaded.completed_phases.contains(&1));
    assert!(reloaded.gathered_info["vision"]["problem"].as_str().is_some());

    prd::wizard::clear_wizard_state(&conn).unwrap();
    let cleared = prd::wizard::load_wizard_state(&conn).unwrap();
    assert!(cleared.is_none());

    env::set_current_dir(original_dir).unwrap();
}

// --- Engine PRD Methods ---

#[tokio::test]
async fn test_engine_prd_import_and_list() {
    let _lock = lock();
    let (engine, tmp, original_dir) = setup_engine().await;

    let specs_dir = tmp.path().join("specs");
    std::fs::create_dir_all(&specs_dir).unwrap();
    std::fs::write(
        specs_dir.join("test.md"),
        "# Test\n\nContent here.\n\n## Sub\n\nSub content.\n",
    ).unwrap();

    engine.prd_import("specs").await.unwrap();

    let sections = engine.prd_list().await.unwrap();
    assert_eq!(sections.len(), 2);

    let section = engine.prd_show("1").await.unwrap();
    assert!(section.is_some());
    assert_eq!(section.unwrap().title, "Test");

    let results = engine.prd_search("content").await.unwrap();
    assert!(!results.is_empty());

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_engine_prd_term_methods() {
    let _lock = lock();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let id = engine.prd_term_add("DIAL", "[]", "Deterministic Iterative Agent Loop", "acronym", None).await.unwrap();
    assert!(id > 0);

    let terms = engine.prd_term_list(None).await.unwrap();
    assert_eq!(terms.len(), 1);
    assert_eq!(terms[0].canonical, "DIAL");

    let results = engine.prd_term_search("Deterministic").await.unwrap();
    assert!(!results.is_empty());

    env::set_current_dir(original_dir).unwrap();
}

// --- Template Tests ---

#[tokio::test]
async fn test_templates_available() {
    let templates = prd::templates::list_templates();
    assert!(templates.len() >= 4);

    assert!(prd::templates::get_template("spec").is_some());
    assert!(prd::templates::get_template("architecture").is_some());
    assert!(prd::templates::get_template("api").is_some());
    assert!(prd::templates::get_template("mvp").is_some());
    assert!(prd::templates::get_template("nonexistent").is_none());

    let spec = prd::templates::get_template("spec").unwrap();
    assert!(!spec.sections.is_empty());
    assert!(!spec.description.is_empty());
}

// --- Migration 10: prd_section_id ---

#[tokio::test]
async fn test_task_prd_section_id_field() {
    let _lock = lock();
    let (engine, _tmp, original_dir) = setup_engine().await;

    let id = engine.task_add("Test task", 5, None).await.unwrap();
    let task = engine.task_get(id).await.unwrap();
    assert!(task.prd_section_id.is_none());

    env::set_current_dir(original_dir).unwrap();
}

// --- Phase 7: Build/Test Config Writing ---

#[tokio::test]
async fn test_apply_build_test_config_writes_config_and_steps() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    let config_data = json!({
        "build_cmd": "cargo build --release",
        "test_cmd": "cargo test",
        "build_timeout": 300,
        "test_timeout": 120,
        "pipeline_steps": [
            {"name": "lint", "command": "cargo clippy", "order": 1, "required": true, "timeout": 60},
            {"name": "build", "command": "cargo build", "order": 2, "required": true, "timeout": 300},
            {"name": "test", "command": "cargo test", "order": 3, "required": true, "timeout": 120},
            {"name": "docs", "command": "cargo doc", "order": 4, "required": false, "timeout": 90}
        ],
        "rationale": "Standard Rust pipeline"
    });

    let (build_cmd, test_cmd, steps_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data).unwrap();

    assert_eq!(build_cmd, "cargo build --release");
    assert_eq!(test_cmd, "cargo test");
    assert_eq!(steps_count, 4);

    // Verify config values were written via config_set
    let stored_build = dial_core::config::config_get("build_cmd").unwrap();
    assert_eq!(stored_build, Some("cargo build --release".to_string()));

    let stored_test = dial_core::config::config_get("test_cmd").unwrap();
    assert_eq!(stored_test, Some("cargo test".to_string()));

    let stored_build_timeout = dial_core::config::config_get("build_timeout").unwrap();
    assert_eq!(stored_build_timeout, Some("300".to_string()));

    let stored_test_timeout = dial_core::config::config_get("test_timeout").unwrap();
    assert_eq!(stored_test_timeout, Some("120".to_string()));

    // Verify validation_steps were inserted
    let mut stmt = phase_conn
        .prepare("SELECT name, command, sort_order, required, timeout_secs FROM validation_steps ORDER BY sort_order")
        .unwrap();
    let steps: Vec<(String, String, i32, i32, Option<i64>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(steps.len(), 4);
    assert_eq!(steps[0].0, "lint");
    assert_eq!(steps[0].1, "cargo clippy");
    assert_eq!(steps[0].2, 1); // sort_order
    assert_eq!(steps[0].3, 1); // required = true
    assert_eq!(steps[0].4, Some(60)); // timeout
    assert_eq!(steps[3].0, "docs");
    assert_eq!(steps[3].3, 0); // required = false

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_build_test_config_defaults() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // Minimal JSON — missing optional fields should use defaults
    let config_data = json!({
        "build_cmd": "make",
        "test_cmd": "make test"
    });

    let (build_cmd, test_cmd, steps_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data).unwrap();

    assert_eq!(build_cmd, "make");
    assert_eq!(test_cmd, "make test");
    assert_eq!(steps_count, 0); // No pipeline_steps provided

    // Timeouts should default to 600
    let stored_build_timeout = dial_core::config::config_get("build_timeout").unwrap();
    assert_eq!(stored_build_timeout, Some("600".to_string()));

    let stored_test_timeout = dial_core::config::config_get("test_timeout").unwrap();
    assert_eq!(stored_test_timeout, Some("600".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

// --- Phase 8: Iteration Mode Config Writing ---

#[tokio::test]
async fn test_apply_iteration_mode_writes_config() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    let mode_data = json!({
        "recommended_mode": "autonomous",
        "review_interval": null,
        "ai_cli": "claude",
        "subagent_timeout": 1800,
        "rationale": "Simple project with low complexity"
    });

    let mode = prd::wizard::apply_iteration_mode(&phase_conn, &mode_data).unwrap();

    assert_eq!(mode, "autonomous");

    // Verify config values were written
    let stored_mode = dial_core::config::config_get("iteration_mode").unwrap();
    assert_eq!(stored_mode, Some("autonomous".to_string()));

    let stored_cli = dial_core::config::config_get("ai_cli").unwrap();
    assert_eq!(stored_cli, Some("claude".to_string()));

    let stored_timeout = dial_core::config::config_get("subagent_timeout").unwrap();
    assert_eq!(stored_timeout, Some("1800".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_iteration_mode_review_every_builds_mode_string() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    let mode_data = json!({
        "recommended_mode": "review_every",
        "review_interval": 3,
        "ai_cli": "codex",
        "subagent_timeout": 900,
        "rationale": "Medium complexity, review every 3 tasks"
    });

    let mode = prd::wizard::apply_iteration_mode(&phase_conn, &mode_data).unwrap();

    assert_eq!(mode, "review_every:3");

    let stored_mode = dial_core::config::config_get("iteration_mode").unwrap();
    assert_eq!(stored_mode, Some("review_every:3".to_string()));

    let stored_cli = dial_core::config::config_get("ai_cli").unwrap();
    assert_eq!(stored_cli, Some("codex".to_string()));

    let stored_timeout = dial_core::config::config_get("subagent_timeout").unwrap();
    assert_eq!(stored_timeout, Some("900".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_iteration_mode_defaults() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // Minimal JSON — missing optional fields should use defaults
    let mode_data = json!({
        "rationale": "defaults test"
    });

    let mode = prd::wizard::apply_iteration_mode(&phase_conn, &mode_data).unwrap();

    // Default mode is "autonomous"
    assert_eq!(mode, "autonomous");

    let stored_mode = dial_core::config::config_get("iteration_mode").unwrap();
    assert_eq!(stored_mode, Some("autonomous".to_string()));

    // Default ai_cli is "claude"
    let stored_cli = dial_core::config::config_get("ai_cli").unwrap();
    assert_eq!(stored_cli, Some("claude".to_string()));

    // Default subagent_timeout is 1800
    let stored_timeout = dial_core::config::config_get("subagent_timeout").unwrap();
    assert_eq!(stored_timeout, Some("1800".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_iteration_mode_review_each() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    let mode_data = json!({
        "recommended_mode": "review_each",
        "ai_cli": "gemini",
        "subagent_timeout": 3600,
        "rationale": "Complex project, review each task"
    });

    let mode = prd::wizard::apply_iteration_mode(&phase_conn, &mode_data).unwrap();

    assert_eq!(mode, "review_each");

    let stored_mode = dial_core::config::config_get("iteration_mode").unwrap();
    assert_eq!(stored_mode, Some("review_each".to_string()));

    let stored_cli = dial_core::config::config_get("ai_cli").unwrap();
    assert_eq!(stored_cli, Some("gemini".to_string()));

    let stored_timeout = dial_core::config::config_get("subagent_timeout").unwrap();
    assert_eq!(stored_timeout, Some("3600".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

// --- Load Existing Doc ---

#[tokio::test]
async fn test_load_existing_doc() {
    let _lock = lock();
    let original_dir = env::current_dir().unwrap();
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();

    let doc_path = tmp.path().join("existing.md");
    std::fs::write(&doc_path, "# My Existing Doc\n\nSome existing content.\n").unwrap();

    let content = prd::wizard::load_existing_doc(&doc_path.to_string_lossy()).unwrap();
    assert!(content.contains("My Existing Doc"));
    assert!(content.contains("existing content"));

    let result = prd::wizard::load_existing_doc("/tmp/nonexistent_wizard_doc.md");
    assert!(result.is_err());

    env::set_current_dir(original_dir).unwrap();
}
