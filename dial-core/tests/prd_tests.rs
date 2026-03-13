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

    let (build_cmd, test_cmd, steps_count, _test_tasks_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &[]).unwrap();

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

    let (build_cmd, test_cmd, steps_count, _test_tasks_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &[]).unwrap();

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

// --- Phase 9: Launch Summary ---

#[tokio::test]
async fn test_run_wizard_phase_9_writes_launch_ready() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();

    // Set up config values as if phases 7 and 8 already ran
    dial_core::config::config_set("build_cmd", "cargo build").unwrap();
    dial_core::config::config_set("test_cmd", "cargo test").unwrap();
    dial_core::config::config_set("iteration_mode", "autonomous").unwrap();
    dial_core::config::config_set("ai_cli", "claude").unwrap();

    // Add some tasks so task_count is non-zero
    let phase_conn = dial_core::get_db(Some("test")).unwrap();
    phase_conn
        .execute(
            "INSERT INTO tasks (description, priority, status) VALUES (?1, ?2, ?3)",
            rusqlite::params!["Task one", 5, "pending"],
        )
        .unwrap();
    phase_conn
        .execute(
            "INSERT INTO tasks (description, priority, status) VALUES (?1, ?2, ?3)",
            rusqlite::params!["Task two", 3, "pending"],
        )
        .unwrap();

    // Build wizard state with prior gathered_info
    let mut state = prd::wizard::WizardState::new("spec");
    state.set_phase_data(
        prd::wizard::WizardPhase::Vision,
        json!({ "project_name": "TestProject", "problem": "testing" }),
    );
    // Mark phases 1-8 complete
    for phase_num in 1..=8 {
        let _phase = prd::wizard::WizardPhase::from_i32(phase_num).unwrap();
        if !state.completed_phases.contains(&phase_num) {
            state.completed_phases.push(phase_num);
        }
        if phase_num == 8 {
            state.current_phase = prd::wizard::WizardPhase::Launch;
        }
    }
    prd::wizard::save_wizard_state(&prd_conn, &state).unwrap();

    // Run phase 9
    let (project_name, task_count) =
        prd::wizard::run_wizard_phase_9(&prd_conn, &mut state).unwrap();

    assert_eq!(project_name, "TestProject");
    assert_eq!(task_count, 2);

    // Verify launch_ready flag is written in gathered_info
    assert_eq!(
        state.gathered_info["launch"]["launch_ready"].as_bool(),
        Some(true)
    );
    assert_eq!(
        state.gathered_info["launch"]["project_name"].as_str(),
        Some("TestProject")
    );
    assert_eq!(state.gathered_info["launch"]["task_count"].as_u64(), Some(2));
    assert_eq!(
        state.gathered_info["launch"]["build_cmd"].as_str(),
        Some("cargo build")
    );

    // Verify phase 9 is in completed_phases
    assert!(state.completed_phases.contains(&9));

    // Verify state persisted to DB
    let reloaded = prd::wizard::load_wizard_state(&prd_conn).unwrap().unwrap();
    assert!(reloaded.completed_phases.contains(&9));
    assert_eq!(
        reloaded.gathered_info["launch"]["launch_ready"].as_bool(),
        Some(true)
    );

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_run_wizard_phase_9_skips_if_already_complete() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();

    let mut state = prd::wizard::WizardState::new("spec");
    state.set_phase_data(
        prd::wizard::WizardPhase::Vision,
        json!({ "project_name": "SkipProject" }),
    );
    // Mark phase 9 already complete
    state.completed_phases.push(9);
    prd::wizard::save_wizard_state(&prd_conn, &state).unwrap();

    let (project_name, task_count) =
        prd::wizard::run_wizard_phase_9(&prd_conn, &mut state).unwrap();

    assert_eq!(project_name, "SkipProject");
    assert_eq!(task_count, 0);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_run_wizard_phase_9_defaults_when_no_vision() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();

    // No vision data, no config set
    let mut state = prd::wizard::WizardState::new("spec");
    prd::wizard::save_wizard_state(&prd_conn, &state).unwrap();

    let (project_name, task_count) =
        prd::wizard::run_wizard_phase_9(&prd_conn, &mut state).unwrap();

    assert_eq!(project_name, "Unknown");
    assert_eq!(task_count, 0);

    // Verify defaults show "(not set)" for unconfigured values
    assert_eq!(
        state.gathered_info["launch"]["build_cmd"].as_str(),
        Some("(not set)")
    );

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

// =============================================================================
// WizardPhase enum tests
// =============================================================================

#[test]
fn test_wizard_phase_from_i32_all_values() {
    use prd::wizard::WizardPhase;

    assert_eq!(WizardPhase::from_i32(1), Some(WizardPhase::Vision));
    assert_eq!(WizardPhase::from_i32(2), Some(WizardPhase::Functionality));
    assert_eq!(WizardPhase::from_i32(3), Some(WizardPhase::Technical));
    assert_eq!(WizardPhase::from_i32(4), Some(WizardPhase::GapAnalysis));
    assert_eq!(WizardPhase::from_i32(5), Some(WizardPhase::Generate));
    assert_eq!(WizardPhase::from_i32(6), Some(WizardPhase::TaskReview));
    assert_eq!(WizardPhase::from_i32(7), Some(WizardPhase::BuildTestConfig));
    assert_eq!(WizardPhase::from_i32(8), Some(WizardPhase::IterationMode));
    assert_eq!(WizardPhase::from_i32(9), Some(WizardPhase::Launch));
}

#[test]
fn test_wizard_phase_from_i32_invalid_values() {
    use prd::wizard::WizardPhase;

    assert_eq!(WizardPhase::from_i32(0), None);
    assert_eq!(WizardPhase::from_i32(-1), None);
    assert_eq!(WizardPhase::from_i32(10), None);
    assert_eq!(WizardPhase::from_i32(100), None);
    assert_eq!(WizardPhase::from_i32(i32::MIN), None);
    assert_eq!(WizardPhase::from_i32(i32::MAX), None);
}

#[test]
fn test_wizard_phase_name_all_values() {
    use prd::wizard::WizardPhase;

    assert_eq!(WizardPhase::Vision.name(), "Vision");
    assert_eq!(WizardPhase::Functionality.name(), "Functionality");
    assert_eq!(WizardPhase::Technical.name(), "Technical");
    assert_eq!(WizardPhase::GapAnalysis.name(), "Gap Analysis");
    assert_eq!(WizardPhase::Generate.name(), "Generate");
    assert_eq!(WizardPhase::TaskReview.name(), "Task Review");
    assert_eq!(WizardPhase::BuildTestConfig.name(), "Build & Test Config");
    assert_eq!(WizardPhase::IterationMode.name(), "Iteration Mode");
    assert_eq!(WizardPhase::Launch.name(), "Launch");
}

#[test]
fn test_wizard_phase_next_chain() {
    use prd::wizard::WizardPhase;

    // Vision -> Functionality -> Technical -> GapAnalysis -> Generate
    // -> TaskReview -> BuildTestConfig -> IterationMode -> Launch
    let mut phase = WizardPhase::Vision;
    let expected = [
        WizardPhase::Functionality,
        WizardPhase::Technical,
        WizardPhase::GapAnalysis,
        WizardPhase::Generate,
        WizardPhase::TaskReview,
        WizardPhase::BuildTestConfig,
        WizardPhase::IterationMode,
        WizardPhase::Launch,
    ];
    for expected_next in &expected {
        let next = phase.next().expect(&format!("{:?} should have a next phase", phase));
        assert_eq!(next, *expected_next);
        phase = next;
    }
}

#[test]
fn test_wizard_phase_next_last_returns_none() {
    use prd::wizard::WizardPhase;

    assert_eq!(WizardPhase::Launch.next(), None);
}

#[test]
fn test_wizard_phase_round_trip() {
    use prd::wizard::WizardPhase;

    // Every phase's integer discriminant round-trips through from_i32
    let all_phases = [
        WizardPhase::Vision,
        WizardPhase::Functionality,
        WizardPhase::Technical,
        WizardPhase::GapAnalysis,
        WizardPhase::Generate,
        WizardPhase::TaskReview,
        WizardPhase::BuildTestConfig,
        WizardPhase::IterationMode,
        WizardPhase::Launch,
    ];
    for (i, phase) in all_phases.iter().enumerate() {
        let val = (i + 1) as i32;
        assert_eq!(*phase as i32, val);
        assert_eq!(WizardPhase::from_i32(val), Some(*phase));
    }
}

// =============================================================================
// Phase 6 prompt builder tests
// =============================================================================

#[test]
fn test_build_task_review_prompt_with_tasks() {
    let tasks: Vec<(i64, String, i32, Option<String>)> = vec![
        (1, "Set up project skeleton".to_string(), 1, Some("1.1".to_string())),
        (2, "Implement auth module".to_string(), 2, Some("2.1".to_string())),
        (3, "Add database layer".to_string(), 3, None),
    ];
    let gathered_info = json!({
        "vision": {"project_name": "TestApp", "problem": "testing"},
        "functionality": {"mvp_features": ["auth", "db"]}
    });

    let prompt = prd::wizard::build_task_review_prompt(&tasks, &gathered_info);

    // Should include each task with its ID, priority, section, and description
    assert!(prompt.contains("[#1]"), "prompt should include task ID 1");
    assert!(prompt.contains("P1"), "prompt should include priority P1");
    assert!(prompt.contains("section: 1.1"), "prompt should include section 1.1");
    assert!(prompt.contains("Set up project skeleton"));
    assert!(prompt.contains("[#2]"));
    assert!(prompt.contains("Implement auth module"));
    assert!(prompt.contains("[#3]"));
    assert!(prompt.contains("section: none"), "task without section should show 'none'");

    // Should include PRD context
    assert!(prompt.contains("Full PRD Context"));
    assert!(prompt.contains("TestApp"));

    // Should include review instructions
    assert!(prompt.contains("0-based indices"));
    assert!(prompt.contains("Respond ONLY with valid JSON"));
}

#[test]
fn test_build_task_review_prompt_empty_tasks() {
    let tasks: Vec<(i64, String, i32, Option<String>)> = vec![];
    let gathered_info = json!({});

    let prompt = prd::wizard::build_task_review_prompt(&tasks, &gathered_info);

    assert!(prompt.contains("No tasks have been generated yet."));
    // Empty gathered_info should not include PRD context section
    assert!(!prompt.contains("Full PRD Context"));
}

#[test]
fn test_build_task_review_prompt_no_gathered_info() {
    let tasks = vec![
        (1, "A task".to_string(), 1, None),
    ];
    let gathered_info = json!({});

    let prompt = prd::wizard::build_task_review_prompt(&tasks, &gathered_info);

    assert!(prompt.contains("[#1]"));
    assert!(prompt.contains("A task"));
    assert!(!prompt.contains("Full PRD Context"));
}

// =============================================================================
// Phase 7 prompt builder tests
// =============================================================================

#[test]
fn test_build_build_test_config_prompt_with_technical() {
    let gathered_info = json!({
        "vision": {"project_name": "RustApp"},
        "technical": {
            "languages": ["Rust"],
            "frameworks": ["Actix Web"],
            "database": "SQLite",
            "constraints": ["must run offline"]
        }
    });

    let prompt = prd::wizard::build_build_test_config_prompt(&gathered_info, &[]);

    assert!(prompt.contains("Technical Details (from Phase 3)"));
    assert!(prompt.contains("Rust"));
    assert!(prompt.contains("Actix Web"));
    assert!(prompt.contains("Full PRD Context"));
    assert!(prompt.contains("pipeline_steps"));
    assert!(prompt.contains("Respond ONLY with valid JSON"));
}

#[test]
fn test_build_build_test_config_prompt_no_technical() {
    let gathered_info = json!({
        "vision": {"project_name": "SimpleApp"}
    });

    let prompt = prd::wizard::build_build_test_config_prompt(&gathered_info, &[]);

    assert!(prompt.contains("No technical details available from prior phases."));
    // Should still include PRD context from gathered_info
    assert!(prompt.contains("Full PRD Context"));
}

#[test]
fn test_build_build_test_config_prompt_empty_gathered_info() {
    let gathered_info = json!({});

    let prompt = prd::wizard::build_build_test_config_prompt(&gathered_info, &[]);

    assert!(prompt.contains("No technical details available from prior phases."));
    assert!(!prompt.contains("Full PRD Context"));
}

// =============================================================================
// Phase 8 prompt builder tests
// =============================================================================

#[test]
fn test_build_iteration_mode_prompt_with_complexity() {
    let gathered_info = json!({
        "vision": {"project_name": "ComplexApp"},
        "functionality": {"mvp_features": ["auth", "billing", "notifications"]},
        "technical": {
            "integrations": ["Stripe", "SendGrid"],
            "constraints": ["PCI compliance", "GDPR"]
        },
        "gap_analysis": {"gaps": ["error handling", "rate limiting", "logging"]}
    });

    let prompt = prd::wizard::build_iteration_mode_prompt(&gathered_info, 25);

    assert!(prompt.contains("ComplexApp"));
    assert!(prompt.contains("Pending tasks: 25"));
    assert!(prompt.contains("MVP features: 3"));
    assert!(prompt.contains("External integrations: 2"));
    assert!(prompt.contains("Constraints: 2"));
    assert!(prompt.contains("Identified gaps: 3"));
    assert!(prompt.contains("autonomous"));
    assert!(prompt.contains("review_every"));
    assert!(prompt.contains("review_each"));
    assert!(prompt.contains("Respond ONLY with valid JSON"));
}

#[test]
fn test_build_iteration_mode_prompt_minimal() {
    let gathered_info = json!({});

    let prompt = prd::wizard::build_iteration_mode_prompt(&gathered_info, 0);

    assert!(prompt.contains("unknown")); // no vision.project_name
    assert!(prompt.contains("Pending tasks: 0"));
    // No complexity indicators section when no data
    assert!(!prompt.contains("Complexity Indicators"));
    assert!(!prompt.contains("Full PRD Context"));
}

#[test]
fn test_build_iteration_mode_prompt_partial_complexity() {
    let gathered_info = json!({
        "vision": {"project_name": "MediumApp"},
        "functionality": {"mvp_features": ["search", "export"]}
        // No technical or gap_analysis
    });

    let prompt = prd::wizard::build_iteration_mode_prompt(&gathered_info, 10);

    assert!(prompt.contains("MediumApp"));
    assert!(prompt.contains("Pending tasks: 10"));
    assert!(prompt.contains("MVP features: 2"));
    // Should not have complexity indicator lines for integrations, constraints, or gaps
    assert!(!prompt.contains("External integrations:"));
    assert!(!prompt.contains("- Constraints:"));
    assert!(!prompt.contains("Identified gaps:"));
}

// =============================================================================
// Phase 6 JSON response parsing tests (apply_task_review)
// =============================================================================

#[tokio::test]
async fn test_apply_task_review_valid_response() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // Seed some existing tasks (as if phase 5 generated them)
    phase_conn.execute(
        "INSERT INTO tasks (description, priority, status) VALUES (?1, ?2, ?3)",
        rusqlite::params!["Old task one", 1, "pending"],
    ).unwrap();
    phase_conn.execute(
        "INSERT INTO tasks (description, priority, status) VALUES (?1, ?2, ?3)",
        rusqlite::params!["Old task two", 2, "pending"],
    ).unwrap();

    let review_data = json!({
        "tasks": [
            {"description": "Set up project", "priority": 1, "spec_section": "1.0", "depends_on": [], "rationale": "foundation"},
            {"description": "Add database", "priority": 2, "spec_section": "2.0", "depends_on": [0], "rationale": "needs project"},
            {"description": "Implement API", "priority": 3, "spec_section": "3.0", "depends_on": [0, 1], "rationale": "needs db"}
        ],
        "removed": [
            {"original": "Old task two", "reason": "redundant"}
        ],
        "added": [
            {"description": "Implement API", "reason": "missing from original"}
        ]
    });

    let (kept, added, removed) = prd::wizard::apply_task_review(&phase_conn, &review_data).unwrap();

    assert_eq!(kept, 2);   // 3 tasks - 1 added = 2 kept
    assert_eq!(added, 1);
    assert_eq!(removed, 1);

    // Verify old tasks were deleted and new ones inserted
    let count: i64 = phase_conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status = 'pending'", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(count, 3);

    // Verify descriptions
    let mut stmt = phase_conn.prepare(
        "SELECT description FROM tasks WHERE status = 'pending' ORDER BY priority"
    ).unwrap();
    let descs: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap()
        .collect::<std::result::Result<Vec<_>, _>>().unwrap();
    assert_eq!(descs, vec!["Set up project", "Add database", "Implement API"]);

    // Verify prd_section_id was set
    let section: Option<String> = phase_conn.query_row(
        "SELECT prd_section_id FROM tasks WHERE description = 'Set up project'",
        [], |r| r.get(0)
    ).unwrap();
    assert_eq!(section, Some("1.0".to_string()));

    // Verify dependency relationships were created
    let dep_count: i64 = phase_conn.query_row(
        "SELECT COUNT(*) FROM task_dependencies", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(dep_count, 3); // task 2 depends on 0; task 3 depends on 0 and 1

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_task_review_missing_tasks_array() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // Response with no "tasks" key should error
    let review_data = json!({
        "removed": [],
        "added": []
    });

    let result = prd::wizard::apply_task_review(&phase_conn, &review_data);
    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("tasks"), "error should mention missing 'tasks': {}", err_msg);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_task_review_malformed_task_fields() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // Tasks with missing/malformed fields should use defaults
    let review_data = json!({
        "tasks": [
            {},  // no description, no priority, no section, no depends_on
            {"description": "Has desc only"},
            {"priority": 99}  // has priority but no description
        ],
        "removed": null,  // null instead of array
        "added": "not an array"  // wrong type
    });

    let (kept, added, removed) = prd::wizard::apply_task_review(&phase_conn, &review_data).unwrap();

    // removed = 0 (null is not an array), added = 0 (string is not an array)
    assert_eq!(removed, 0);
    assert_eq!(added, 0);
    assert_eq!(kept, 3); // 3 tasks - 0 added

    // Verify tasks inserted with defaults
    let mut stmt = phase_conn.prepare(
        "SELECT description, priority FROM tasks WHERE status = 'pending' ORDER BY rowid"
    ).unwrap();
    let rows: Vec<(String, i32)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap().collect::<std::result::Result<Vec<_>, _>>().unwrap();

    assert_eq!(rows[0].0, "Untitled task");  // default description
    assert_eq!(rows[0].1, 5);                // default priority
    assert_eq!(rows[1].0, "Has desc only");
    assert_eq!(rows[1].1, 5);                // default priority
    assert_eq!(rows[2].0, "Untitled task");  // no description key
    assert_eq!(rows[2].1, 99);               // explicit priority

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_task_review_self_dependency_ignored() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // A task that depends on itself should be silently ignored
    let review_data = json!({
        "tasks": [
            {"description": "Task A", "priority": 1, "depends_on": [0]}
        ]
    });

    let (kept, _added, _removed) = prd::wizard::apply_task_review(&phase_conn, &review_data).unwrap();
    assert_eq!(kept, 1);

    // Self-dependency should not be inserted
    let dep_count: i64 = phase_conn.query_row(
        "SELECT COUNT(*) FROM task_dependencies", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(dep_count, 0);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_task_review_out_of_bounds_dependency_ignored() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // Dependency index beyond array length should be silently ignored
    let review_data = json!({
        "tasks": [
            {"description": "Task A", "priority": 1, "depends_on": [5, 99]}
        ]
    });

    prd::wizard::apply_task_review(&phase_conn, &review_data).unwrap();

    let dep_count: i64 = phase_conn.query_row(
        "SELECT COUNT(*) FROM task_dependencies", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(dep_count, 0);

    env::set_current_dir(original_dir).unwrap();
}

// =============================================================================
// Phase 7 JSON response parsing tests (apply_build_test_config)
// =============================================================================

#[tokio::test]
async fn test_apply_build_test_config_missing_commands() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // No build_cmd or test_cmd — should default to empty strings
    let config_data = json!({
        "rationale": "minimal"
    });

    let (build_cmd, test_cmd, steps_count, _test_tasks_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &[]).unwrap();

    assert_eq!(build_cmd, "");
    assert_eq!(test_cmd, "");
    assert_eq!(steps_count, 0);

    let stored_build = dial_core::config::config_get("build_cmd").unwrap();
    assert_eq!(stored_build, Some("".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_build_test_config_malformed_pipeline_steps() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // Pipeline steps with missing/malformed fields should use defaults
    let config_data = json!({
        "build_cmd": "make",
        "test_cmd": "make test",
        "pipeline_steps": [
            {},  // all fields missing
            {"name": "lint"},  // only name
            {"command": "cargo test", "required": false, "timeout": 42}  // no name, no order
        ]
    });

    let (_, _, steps_count, _) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &[]).unwrap();

    assert_eq!(steps_count, 3);

    let mut stmt = phase_conn.prepare(
        "SELECT name, command, sort_order, required, timeout_secs FROM validation_steps ORDER BY rowid"
    ).unwrap();
    let steps: Vec<(String, String, i32, i32, Option<i64>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    // First step: all defaults
    assert_eq!(steps[0].0, "step");      // default name
    assert_eq!(steps[0].1, "");          // default command
    assert_eq!(steps[0].2, 0);           // default order
    assert_eq!(steps[0].3, 1);           // default required = true
    assert_eq!(steps[0].4, None);        // default timeout = None

    // Second step: only name provided
    assert_eq!(steps[1].0, "lint");
    assert_eq!(steps[1].1, "");          // default command

    // Third step: explicit values
    assert_eq!(steps[2].0, "step");      // default name
    assert_eq!(steps[2].1, "cargo test");
    assert_eq!(steps[2].3, 0);           // required = false
    assert_eq!(steps[2].4, Some(42));    // explicit timeout

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_build_test_config_empty_pipeline_steps() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    let config_data = json!({
        "build_cmd": "go build",
        "test_cmd": "go test ./...",
        "pipeline_steps": []
    });

    let (_, _, steps_count, _) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &[]).unwrap();

    assert_eq!(steps_count, 0);

    env::set_current_dir(original_dir).unwrap();
}

// =============================================================================
// Phase 8 JSON response parsing tests (apply_iteration_mode)
// =============================================================================

#[tokio::test]
async fn test_apply_iteration_mode_review_every_no_interval_defaults_to_5() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // review_every with no interval should default to 5
    let mode_data = json!({
        "recommended_mode": "review_every",
        "review_interval": null
    });

    let mode = prd::wizard::apply_iteration_mode(&phase_conn, &mode_data).unwrap();
    assert_eq!(mode, "review_every:5");

    let stored_mode = dial_core::config::config_get("iteration_mode").unwrap();
    assert_eq!(stored_mode, Some("review_every:5".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_iteration_mode_unknown_mode_passes_through() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // An unexpected mode value passes through as-is
    let mode_data = json!({
        "recommended_mode": "custom_mode",
        "ai_cli": "claude",
        "subagent_timeout": 600
    });

    let mode = prd::wizard::apply_iteration_mode(&phase_conn, &mode_data).unwrap();
    assert_eq!(mode, "custom_mode");

    let stored_mode = dial_core::config::config_get("iteration_mode").unwrap();
    assert_eq!(stored_mode, Some("custom_mode".to_string()));

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_iteration_mode_wrong_types_use_defaults() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    // All values are wrong types — should fall back to defaults
    let mode_data = json!({
        "recommended_mode": 123,
        "review_interval": "not a number",
        "ai_cli": 456,
        "subagent_timeout": "big"
    });

    let mode = prd::wizard::apply_iteration_mode(&phase_conn, &mode_data).unwrap();

    // recommended_mode is not a string → default "autonomous"
    assert_eq!(mode, "autonomous");

    // ai_cli is not a string → default "claude"
    let stored_cli = dial_core::config::config_get("ai_cli").unwrap();
    assert_eq!(stored_cli, Some("claude".to_string()));

    // subagent_timeout is not a number → default 1800
    let stored_timeout = dial_core::config::config_get("subagent_timeout").unwrap();
    assert_eq!(stored_timeout, Some("1800".to_string()));

    env::set_current_dir(original_dir).unwrap();
}
