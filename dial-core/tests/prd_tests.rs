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

fn safe_current_dir() -> std::path::PathBuf {
    env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
}

/// Helper: create an Engine in a temp directory.
async fn setup_engine() -> (Engine, TempDir, std::path::PathBuf) {
    let original_dir = safe_current_dir();
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
        assert!(
            s.parent_id.is_some(),
            "h2 section '{}' should have a parent_id",
            s.title
        );
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
    )
    .unwrap();

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

    let id = prd::prd_add_term(
        &conn,
        "API",
        "[\"api\", \"Rest API\"]",
        "Application Programming Interface",
        "technical",
        None,
    )
    .unwrap();
    assert!(id > 0);

    prd::prd_add_term(
        &conn,
        "PRD",
        "[\"prd\"]",
        "Product Requirements Document",
        "domain",
        Some("overview"),
    )
    .unwrap();
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

    prd::prd_insert_section(
        &conn,
        "1",
        "Overview",
        None,
        1,
        0,
        "This is the overview.",
        4,
    )
    .unwrap();
    prd::prd_insert_section(
        &conn,
        "1.1",
        "Goals",
        Some("1"),
        2,
        1,
        "Our goals are clear.",
        4,
    )
    .unwrap();
    prd::prd_insert_section(
        &conn,
        "1.2",
        "Non-Goals",
        Some("1"),
        2,
        2,
        "What we won't do.",
        4,
    )
    .unwrap();

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
    prd::prd_insert_section(
        &conn,
        "1",
        "Authentication",
        None,
        1,
        0,
        "Users authenticate via OAuth2 tokens and bearer authentication.",
        7,
    )
    .unwrap();
    prd::prd_insert_section(
        &conn,
        "2",
        "Data Model",
        None,
        1,
        1,
        "The data model uses SQLite with FTS5 for search.",
        9,
    )
    .unwrap();
    drop(conn);

    // Add a task whose description matches PRD content
    let task_id = engine
        .task_add("Implement authentication", 1, None)
        .await
        .unwrap();
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
    assert!(
        !context.is_empty(),
        "Context should still include signs and other non-spec content"
    );

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

    prd::wizard::save_wizard_state(&conn, &mut state).unwrap();
    assert!(state.id > 0, "first save should retain the inserted row id");

    let loaded = prd::wizard::load_wizard_state(&conn).unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.template, "mvp");
    assert_eq!(loaded.current_phase, prd::wizard::WizardPhase::Vision);

    state.mark_phase_complete(prd::wizard::WizardPhase::Vision);
    state.set_phase_data(
        prd::wizard::WizardPhase::Vision,
        serde_json::json!({
            "problem": "Users need better specs",
            "target_users": "Developers"
        }),
    );
    prd::wizard::save_wizard_state(&conn, &mut state).unwrap();
    let wizard_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM wizard_state", [], |row| row.get(0))
        .unwrap();
    assert_eq!(wizard_rows, 1, "state updates should not create extra rows");

    let reloaded = prd::wizard::load_wizard_state(&conn).unwrap().unwrap();
    assert_eq!(
        reloaded.current_phase,
        prd::wizard::WizardPhase::Functionality
    );
    assert!(reloaded.completed_phases.contains(&1));
    assert!(reloaded.gathered_info["vision"]["problem"]
        .as_str()
        .is_some());

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
    )
    .unwrap();

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

    let id = engine
        .prd_term_add(
            "DIAL",
            "[]",
            "Deterministic Iterative Agent Loop",
            "acronym",
            None,
        )
        .await
        .unwrap();
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
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
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

#[tokio::test]
async fn test_apply_build_test_config_normalizes_unicode_dash_commands() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    let config_data = json!({
        "build_cmd": "cargo —workspace build",
        "test_cmd": "cargo test —all—features",
        "pipeline_steps": [
            {"name": "build", "command": "cargo —workspace build", "sort_order": 1, "required": true, "timeout": 120},
            {"name": "test", "command": "cargo test —all—features", "sort_order": 2, "required": true, "timeout": 300}
        ]
    });

    let (build_cmd, test_cmd, steps_count, _test_tasks_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &[]).unwrap();

    assert_eq!(build_cmd, "cargo --workspace build");
    assert_eq!(test_cmd, "cargo test --all-features");
    assert_eq!(steps_count, 2);

    let stored_build_cmd = dial_core::config::config_get("build_cmd").unwrap();
    assert_eq!(
        stored_build_cmd,
        Some("cargo --workspace build".to_string())
    );

    let stored_test_cmd = dial_core::config::config_get("test_cmd").unwrap();
    assert_eq!(
        stored_test_cmd,
        Some("cargo test --all-features".to_string())
    );

    let mut stmt = phase_conn
        .prepare("SELECT name, command FROM validation_steps ORDER BY sort_order, id")
        .unwrap();
    let steps: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(steps.len(), 2);
    assert_eq!(
        steps[0],
        ("build".to_string(), "cargo --workspace build".to_string())
    );
    assert_eq!(
        steps[1],
        ("test".to_string(), "cargo test --all-features".to_string())
    );

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_build_test_config_skips_brittle_optional_inline_node_eval_step() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();
    let long_inline_eval = "node -e \"const { spawnSync } = require('child_process'); const payload = JSON.stringify({ title: 'Ship MVP', status: 'todo', tags: ['Bug', ' bug ', 'BUG'] }); const ok = spawnSync(process.execPath, ['src/cli.js'], { input: payload, encoding: 'utf8' }); if (ok.status !== 0) { process.stderr.write(ok.stderr || 'CLI success case failed\\n'); process.exit(ok.status || 1); } if (!ok.stdout.includes('[ ] Ship MVP') || !ok.stdout.includes('bug')) { process.stderr.write('CLI success output did not match expectations\\n'); process.exit(1); }\"";
    let config_data = json!({
        "build_cmd": "npm run build",
        "test_cmd": "npm test",
        "pipeline_steps": [
            {"name": "build", "command": "npm run build", "sort_order": 1, "required": true, "timeout": 300},
            {"name": "cli-smoke", "command": long_inline_eval, "sort_order": 2, "required": false, "timeout": 120}
        ]
    });

    let (_build_cmd, _test_cmd, steps_count, _test_tasks_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &[]).unwrap();

    assert_eq!(steps_count, 1);

    let mut stmt = phase_conn
        .prepare("SELECT name, command FROM validation_steps ORDER BY sort_order, id")
        .unwrap();
    let steps: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].0, "build");
    assert_eq!(steps[0].1, "npm run build");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_build_test_config_skips_optional_inline_node_eval_with_single_quotes() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();
    let quoted_inline_eval = "node -e 'const fs=require(\"fs\"); [\"package.json\",\"src/noteFormatter.js\"].forEach(p=>fs.accessSync(p)); console.log(\"required files present\")'";
    let config_data = json!({
        "build_cmd": "npm run build",
        "test_cmd": "npm test",
        "pipeline_steps": [
            {"name": "build", "command": "npm run build", "sort_order": 1, "required": true, "timeout": 300},
            {"name": "preflight-required-files", "command": quoted_inline_eval, "sort_order": 2, "required": false, "timeout": 60}
        ]
    });

    let (_build_cmd, _test_cmd, steps_count, _test_tasks_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &[]).unwrap();

    assert_eq!(steps_count, 1);

    let mut stmt = phase_conn
        .prepare("SELECT name, command FROM validation_steps ORDER BY sort_order, id")
        .unwrap();
    let steps: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].0, "build");
    assert_eq!(steps[0].1, "npm run build");

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_build_test_config_skips_redundant_test_task_when_feature_owns_coverage() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();
    let feature_description = r#"Finish `src/cli.js` so `node src/cli.js` reads one JSON note from stdin and add CLI coverage in `test/noteFormatter.test.js`."#;
    phase_conn
        .execute(
            "INSERT INTO tasks (description, status, priority) VALUES (?1, 'pending', 3)",
            [feature_description],
        )
        .unwrap();
    let feature_id = phase_conn.last_insert_rowid();

    let feature_tasks = vec![(feature_id, feature_description.to_string(), 3, None)];
    let config_data = json!({
        "build_cmd": "npm run build",
        "test_cmd": "npm test",
        "test_tasks": [
            {
                "description": "Add CLI integration coverage in test/noteFormatter.test.js for valid stdin JSON and invalid JSON handling",
                "depends_on_feature": 0,
                "rationale": "CLI behavior needs subprocess coverage"
            }
        ]
    });

    let (_build_cmd, _test_cmd, _steps_count, test_tasks_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &feature_tasks).unwrap();

    assert_eq!(
        test_tasks_count, 0,
        "Should not create a separate test task when the feature task already owns explicit coverage"
    );

    let pending_tasks: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pending_tasks, 1,
        "Should keep only the original feature task"
    );

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_build_test_config_skips_redundant_test_task_when_existing_test_task_covers_feature(
) {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();
    let feature_description = r#"Implement `src/cli.js` to read one JSON note object from stdin, print the formatted note to stdout using `src/noteFormatter.js`, and exit non-zero with a human-readable error message when stdin contains invalid JSON."#;
    let existing_test_description = r#"Extend `test/noteFormatter.test.js` and, only if required for existing scripts, `package.json` so `npm test` verifies the CLI accepts JSON from stdin, prints the formatted note to stdout, and returns a non-zero exit code with a clear error on invalid JSON while preserving the existing `npm test` and `npm run build` commands."#;
    phase_conn
        .execute(
            "INSERT INTO tasks (description, status, priority) VALUES (?1, 'pending', 3)",
            [feature_description],
        )
        .unwrap();
    let feature_id = phase_conn.last_insert_rowid();
    phase_conn
        .execute(
            "INSERT INTO tasks (description, status, priority) VALUES (?1, 'pending', 4)",
            [existing_test_description],
        )
        .unwrap();
    let existing_test_id = phase_conn.last_insert_rowid();

    let feature_tasks = vec![
        (feature_id, feature_description.to_string(), 3, None),
        (
            existing_test_id,
            existing_test_description.to_string(),
            4,
            None,
        ),
    ];
    let config_data = json!({
        "build_cmd": "npm run build",
        "test_cmd": "npm test",
        "test_tasks": [
            {
                "description": "Add CLI integration tests that execute `node src/cli.js` with stdin payloads: a valid JSON note with `status: 'todo'` and mixed-case duplicate tags prints the formatted note to stdout, and invalid JSON prints a human-readable error to stderr and exits with a non-zero code.",
                "depends_on_feature": 0,
                "rationale": "CLI behavior needs subprocess coverage"
            }
        ]
    });

    let (_build_cmd, _test_cmd, _steps_count, test_tasks_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &feature_tasks).unwrap();

    assert_eq!(
        test_tasks_count, 0,
        "Should not create a second CLI test task when phase 6 already added one"
    );

    let pending_tasks: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pending_tasks, 2,
        "Should keep the existing feature and test tasks without inserting another duplicate"
    );

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_build_test_config_folds_manual_browser_verification_into_feature_task() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();
    let feature_description = r#"Add a Compact mode control to `settings.html` and wire `settings.js` to load, display, and persist the `app-settings` `compactMode` value with synced status text."#;
    phase_conn
        .execute(
            "INSERT INTO tasks (description, status, priority, requires_browser_verification)
             VALUES (?1, 'pending', 3, 0)",
            [feature_description],
        )
        .unwrap();
    let feature_id = phase_conn.last_insert_rowid();

    let feature_tasks = vec![(feature_id, feature_description.to_string(), 3, None)];
    let config_data = json!({
        "build_cmd": "npm run build",
        "test_cmd": "npm test",
        "test_tasks": [
            {
                "description": "Manually verify settings.html shows a visible Compact mode checkbox and status text, initializes from the saved app-settings localStorage value, and updates both the status text and localStorage payload when toggled in the browser.",
                "depends_on_feature": 0,
                "rationale": "Visible settings behavior needs browser confirmation"
            }
        ]
    });

    let (_build_cmd, _test_cmd, _steps_count, test_tasks_count) =
        prd::wizard::apply_build_test_config(&phase_conn, &config_data, &feature_tasks).unwrap();

    assert_eq!(
        test_tasks_count, 0,
        "Should not create a separate task for manual browser verification"
    );

    let pending_tasks: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pending_tasks, 1,
        "Should keep only the original feature task"
    );

    let browser_required: i64 = phase_conn
        .query_row(
            "SELECT requires_browser_verification FROM tasks WHERE id = ?1",
            [feature_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        browser_required, 1,
        "Manual browser verification should stay attached to the feature task"
    );

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
async fn test_apply_iteration_mode_preference_overrides_model_choice() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    let mode_data = json!({
        "recommended_mode": "review_every",
        "review_interval": 5,
        "ai_cli": "claude",
        "subagent_timeout": 1200,
        "rationale": "Prefer the current wizard CLI"
    });

    let mode =
        prd::wizard::apply_iteration_mode_with_preference(&phase_conn, &mode_data, Some("copilot"))
            .unwrap();

    assert_eq!(mode, "review_every:5");
    let stored_cli = dial_core::config::config_get("ai_cli").unwrap();
    assert_eq!(stored_cli, Some("copilot".to_string()));

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
    prd::wizard::save_wizard_state(&prd_conn, &mut state).unwrap();

    // Run phase 9
    let summary = prd::wizard::run_wizard_phase_9(&prd_conn, &mut state).unwrap();

    assert_eq!(summary.project_name, "TestProject");
    assert_eq!(summary.task_count, 2);
    assert_eq!(summary.build_cmd, "cargo build");
    assert_eq!(summary.test_cmd, "cargo test");
    assert_eq!(summary.iteration_mode, "autonomous");
    assert_eq!(summary.ai_cli, "claude");

    // Verify launch_ready flag is written in gathered_info
    assert_eq!(
        state.gathered_info["launch"]["launch_ready"].as_bool(),
        Some(true)
    );
    assert_eq!(
        state.gathered_info["launch"]["project_name"].as_str(),
        Some("TestProject")
    );
    assert_eq!(
        state.gathered_info["launch"]["task_count"].as_u64(),
        Some(2)
    );
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
    prd::wizard::save_wizard_state(&prd_conn, &mut state).unwrap();

    let summary = prd::wizard::run_wizard_phase_9(&prd_conn, &mut state).unwrap();

    assert_eq!(summary.project_name, "SkipProject");
    assert_eq!(summary.task_count, 0);

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_run_wizard_phase_9_defaults_when_no_vision() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();

    // No vision data, no config set
    let mut state = prd::wizard::WizardState::new("spec");
    prd::wizard::save_wizard_state(&prd_conn, &mut state).unwrap();

    let summary = prd::wizard::run_wizard_phase_9(&prd_conn, &mut state).unwrap();

    assert_eq!(summary.project_name, "Current Project");
    assert_eq!(summary.task_count, 0);

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
    let original_dir = safe_current_dir();
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
        let next = phase
            .next()
            .expect(&format!("{:?} should have a next phase", phase));
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
        (
            1,
            "Set up project skeleton".to_string(),
            1,
            Some("1.1".to_string()),
        ),
        (
            2,
            "Implement auth module".to_string(),
            2,
            Some("2.1".to_string()),
        ),
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
    assert!(
        prompt.contains("section: 1.1"),
        "prompt should include section 1.1"
    );
    assert!(prompt.contains("Set up project skeleton"));
    assert!(prompt.contains("[#2]"));
    assert!(prompt.contains("Implement auth module"));
    assert!(prompt.contains("[#3]"));
    assert!(
        prompt.contains("section: none"),
        "task without section should show 'none'"
    );

    // Should include the slimmer project context summary
    assert!(prompt.contains("Project Summary"));
    assert!(prompt.contains("TestApp"));
    assert!(!prompt.contains("Full PRD Context"));

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
    // Empty gathered_info should not include project context sections
    assert!(!prompt.contains("Full PRD Context"));
    assert!(!prompt.contains("Project Summary"));
}

#[test]
fn test_build_task_review_prompt_no_gathered_info() {
    let tasks = vec![(1, "A task".to_string(), 1, None)];
    let gathered_info = json!({});

    let prompt = prd::wizard::build_task_review_prompt(&tasks, &gathered_info);

    assert!(prompt.contains("[#1]"));
    assert!(prompt.contains("A task"));
    assert!(!prompt.contains("Full PRD Context"));
    assert!(!prompt.contains("Project Summary"));
}

#[test]
fn test_build_task_review_prompt_discourages_trailing_verification_tasks() {
    let tasks = vec![(1, "A task".to_string(), 1, None)];
    let gathered_info = json!({});

    let prompt = prd::wizard::build_task_review_prompt(&tasks, &gathered_info);

    assert!(prompt.contains("NO TRAILING VERIFICATION TASKS"));
    assert!(prompt
        .contains("Fold generic acceptance-check-only work into related feature or test tasks"));
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
    assert!(prompt.contains("Project Summary"));
    assert!(!prompt.contains("Full PRD Context"));
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
    // Should still include the project summary from gathered_info
    assert!(prompt.contains("Project Summary"));
    assert!(!prompt.contains("Full PRD Context"));
}

#[test]
fn test_build_build_test_config_prompt_empty_gathered_info() {
    let gathered_info = json!({});

    let prompt = prd::wizard::build_build_test_config_prompt(&gathered_info, &[]);

    assert!(prompt.contains("No technical details available from prior phases."));
    assert!(!prompt.contains("Full PRD Context"));
    assert!(!prompt.contains("Project Summary"));
}

#[test]
fn test_build_build_test_config_prompt_discourages_manual_browser_test_tasks() {
    let prompt = prd::wizard::build_build_test_config_prompt(&json!({}), &[]);

    assert!(prompt.contains(
        "Do NOT emit a separate `test_task` whose only purpose is manual browser verification"
    ));
    assert!(prompt.contains("Keep manual browser verification attached to the feature task"));
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
fn test_build_iteration_mode_prompt_with_current_cli_hint() {
    let gathered_info = json!({
        "vision": {"project_name": "ComplexApp"}
    });

    let prompt = prd::wizard::build_iteration_mode_prompt_with_preference(
        &gathered_info,
        5,
        Some("copilot"),
    );

    assert!(prompt.contains("Current machine-default CLI"));
    assert!(prompt.contains("`copilot`"));
}

#[test]
fn test_build_iteration_mode_prompt_biases_small_local_projects_toward_autonomous() {
    let gathered_info = json!({
        "vision": {"project_name": "Mini Note Formatter"},
        "functionality": {"mvp_features": ["status", "tags", "cli", "tests"]},
        "technical": {"integrations": [], "constraints": ["plain Node.js", "no dependencies"]},
        "gap_analysis": {"gaps": ["stdin edge cases", "error stream behavior"]}
    });

    let prompt = prd::wizard::build_iteration_mode_prompt(&gathered_info, 5);

    assert!(prompt.contains("Autonomy bias:"));
    assert!(prompt.contains("prefer `autonomous`"));
}

#[test]
fn test_build_iteration_mode_prompt_minimal() {
    let gathered_info = json!({});

    let prompt = prd::wizard::build_iteration_mode_prompt(&gathered_info, 0);

    assert!(prompt.contains("current project")); // no vision.project_name
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

#[test]
fn test_apply_autonomous_iteration_override_for_small_local_project() {
    let gathered_info = json!({
        "functionality": {"mvp_features": ["status", "tags", "cli", "tests"]},
        "technical": {"integrations": []},
        "gap_analysis": {"gaps": ["stdin", "errors"]}
    });
    let mode_data = json!({
        "recommended_mode": "review_every",
        "review_interval": 2,
        "rationale": "Needs checkpoints"
    });

    let overridden =
        prd::wizard::apply_autonomous_iteration_override(&gathered_info, 5, &mode_data);

    assert_eq!(
        overridden.get("recommended_mode").and_then(|v| v.as_str()),
        Some("autonomous")
    );
    assert!(overridden
        .get("review_interval")
        .is_some_and(|value| value.is_null()));
}

#[test]
fn test_apply_autonomous_iteration_override_preserves_larger_project_review_mode() {
    let gathered_info = json!({
        "functionality": {"mvp_features": ["auth", "billing", "notifications", "search", "export", "reports"]},
        "technical": {"integrations": ["Stripe"]},
        "gap_analysis": {"gaps": ["retry", "logging", "monitoring"]}
    });
    let mode_data = json!({
        "recommended_mode": "review_every",
        "review_interval": 2
    });

    let overridden =
        prd::wizard::apply_autonomous_iteration_override(&gathered_info, 8, &mode_data);

    assert_eq!(
        overridden.get("recommended_mode").and_then(|v| v.as_str()),
        Some("review_every")
    );
    assert_eq!(
        overridden.get("review_interval").and_then(|v| v.as_i64()),
        Some(2)
    );
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
    phase_conn
        .execute(
            "INSERT INTO tasks (description, priority, status) VALUES (?1, ?2, ?3)",
            rusqlite::params!["Old task one", 1, "pending"],
        )
        .unwrap();
    phase_conn
        .execute(
            "INSERT INTO tasks (description, priority, status) VALUES (?1, ?2, ?3)",
            rusqlite::params!["Old task two", 2, "pending"],
        )
        .unwrap();

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

    assert_eq!(kept, 2); // 3 tasks - 1 added = 2 kept
    assert_eq!(added, 1);
    assert_eq!(removed, 1);

    // Verify old tasks were deleted and new ones inserted
    let count: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 3);

    // Verify descriptions
    let mut stmt = phase_conn
        .prepare("SELECT description FROM tasks WHERE status = 'pending' ORDER BY priority")
        .unwrap();
    let descs: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        descs,
        vec!["Set up project", "Add database", "Implement API"]
    );

    // Verify prd_section_id was set
    let section: Option<String> = phase_conn
        .query_row(
            "SELECT prd_section_id FROM tasks WHERE description = 'Set up project'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(section, Some("1.0".to_string()));

    // Verify dependency relationships were created
    let dep_count: i64 = phase_conn
        .query_row("SELECT COUNT(*) FROM task_dependencies", [], |r| r.get(0))
        .unwrap();
    assert_eq!(dep_count, 3); // task 2 depends on 0; task 3 depends on 0 and 1

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_task_review_pushes_matching_test_task_after_implementation() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    phase_conn
        .execute(
            "INSERT INTO tasks (description, priority, status) VALUES (?1, ?2, ?3)",
            rusqlite::params!["placeholder", 1, "pending"],
        )
        .unwrap();

    let review_data = json!({
        "tasks": [
            {
                "description": "Add automated tests in `test/noteFormatter.test.js` for status-aware title prefixes and tag normalization, including lowercase conversion, whitespace trimming, duplicate removal, and first-seen order preservation.",
                "priority": 1,
                "spec_section": "6",
                "depends_on": [],
                "rationale": "coverage first"
            },
            {
                "description": "Implement the required note-formatting behavior in `src/noteFormatter.js`: render `[ ]` for status `todo`, render `[x]` for status `done`, render no checkbox when status is missing, and normalize tags by lowercasing, trimming whitespace, removing duplicates, and preserving first-seen order after normalization.",
                "priority": 2,
                "spec_section": "2",
                "depends_on": [],
                "rationale": "implementation"
            }
        ],
        "removed": [],
        "added": []
    });

    prd::wizard::apply_task_review(&phase_conn, &review_data).unwrap();

    let test_priority: i64 = phase_conn
        .query_row(
            "SELECT priority FROM tasks WHERE description LIKE 'Add automated tests in `test/noteFormatter.test.js`%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let impl_priority: i64 = phase_conn
        .query_row(
            "SELECT priority FROM tasks WHERE description LIKE 'Implement the required note-formatting behavior%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        test_priority > impl_priority,
        "test task should be pushed after the matching implementation task"
    );

    let dependency_count: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM task_dependencies td
             JOIN tasks t ON t.id = td.task_id
             JOIN tasks d ON d.id = td.depends_on_id
             WHERE t.description LIKE 'Add automated tests in `test/noteFormatter.test.js`%'
               AND d.description LIKE 'Implement the required note-formatting behavior%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        dependency_count, 1,
        "test task should depend on the matching implementation task"
    );

    env::set_current_dir(original_dir).unwrap();
}

#[tokio::test]
async fn test_apply_task_review_folds_generic_verification_task_into_feature_slice() {
    let _lock = lock();
    let (_engine, _tmp, original_dir) = setup_engine().await;

    let phase_conn = dial_core::get_db(Some("test")).unwrap();

    phase_conn
        .execute(
            "INSERT INTO tasks (description, priority, status) VALUES (?1, ?2, ?3)",
            rusqlite::params!["placeholder", 1, "pending"],
        )
        .unwrap();

    let review_data = json!({
        "tasks": [
            {
                "description": "Update `settings.html` and `settings.js` to provide a visible Compact mode checkbox, keep a status message synced to the saved value, and persist `{ \"compactMode\": true|false }` to the `app-settings` localStorage key.",
                "priority": 1,
                "spec_section": "2.1",
                "depends_on": [],
                "acceptance_criteria": [
                    "settings.html shows a Compact mode checkbox and status text",
                    "toggling the checkbox updates app-settings in localStorage"
                ],
                "requires_browser_verification": true,
                "task_kind": "feature",
                "feature_group": "compact-mode",
                "coverage_mode": "inline"
            },
            {
                "description": "Update `index.html` and `app.js` so the home page reads `app-settings`, shows `Compact mode on` or `Compact mode off`, and toggles the `compact-mode` class on `<body>`.",
                "priority": 2,
                "spec_section": "2.2",
                "depends_on": [0],
                "acceptance_criteria": [
                    "the home page preview reflects the saved compact mode state"
                ],
                "requires_browser_verification": true,
                "task_kind": "feature",
                "feature_group": "compact-mode",
                "coverage_mode": "inline"
            },
            {
                "description": "Add Node-based automated tests in `test/app.test.js` that cover compact mode persistence from the settings page and the home-page preview state after reading saved settings.",
                "priority": 3,
                "spec_section": "6.1",
                "depends_on": [0, 1],
                "acceptance_criteria": [
                    "npm test covers compact mode persistence and preview state"
                ],
                "requires_browser_verification": false,
                "task_kind": "test",
                "feature_group": "compact-mode",
                "coverage_mode": "dedicated"
            },
            {
                "description": "Run the acceptance checks for the compact-mode flow by confirming `npm run build`, `npm test`, and the browser-visible `settings.html` and `index.html` behavior match the requested outcomes.",
                "priority": 4,
                "spec_section": "7.0",
                "depends_on": [0, 1, 2],
                "acceptance_criteria": [
                    "build and test commands pass for the compact mode flow",
                    "browser-visible compact mode behavior matches the requested outcomes"
                ],
                "requires_browser_verification": true,
                "task_kind": "verification",
                "feature_group": "compact-mode",
                "coverage_mode": "none"
            }
        ],
        "removed": [],
        "added": []
    });

    prd::wizard::apply_task_review(&phase_conn, &review_data).unwrap();

    let descriptions: Vec<String> = phase_conn
        .prepare("SELECT description FROM tasks WHERE status = 'pending' ORDER BY priority")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        descriptions.len(),
        3,
        "generic verification task should be folded out"
    );
    assert!(
        descriptions
            .iter()
            .all(|description| !description.contains("Run the acceptance checks")),
        "trailing verification task should be removed"
    );

    let browser_required: i64 = phase_conn
        .query_row(
            "SELECT requires_browser_verification
             FROM tasks
             WHERE description LIKE 'Update `settings.html` and `settings.js`%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(browser_required, 1);

    let criteria_json: String = phase_conn
        .query_row(
            "SELECT acceptance_criteria_json
             FROM tasks
             WHERE description LIKE 'Update `settings.html` and `settings.js`%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let criteria: Vec<String> = serde_json::from_str(&criteria_json).unwrap();
    assert!(
        criteria
            .iter()
            .any(|criterion| criterion.contains("build and test commands pass")),
        "folded verification acceptance criteria should move to the anchor task"
    );

    let dependency_count: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM task_dependencies td
             JOIN tasks t ON t.id = td.task_id
             JOIN tasks d ON d.id = td.depends_on_id
             WHERE t.description LIKE 'Add Node-based automated tests in `test/app.test.js`%'
               AND d.description LIKE 'Update `settings.html` and `settings.js`%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dependency_count, 1);

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
    assert!(
        err_msg.contains("tasks"),
        "error should mention missing 'tasks': {}",
        err_msg
    );

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
    let mut stmt = phase_conn
        .prepare("SELECT description, priority FROM tasks WHERE status = 'pending' ORDER BY rowid")
        .unwrap();
    let rows: Vec<(String, i32)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(rows[0].0, "Untitled task"); // default description
    assert_eq!(rows[0].1, 5); // default priority
    assert_eq!(rows[1].0, "Has desc only");
    assert_eq!(rows[1].1, 5); // default priority
    assert_eq!(rows[2].0, "Untitled task"); // no description key
    assert_eq!(rows[2].1, 99); // explicit priority

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

    let (kept, _added, _removed) =
        prd::wizard::apply_task_review(&phase_conn, &review_data).unwrap();
    assert_eq!(kept, 1);

    // Self-dependency should not be inserted
    let dep_count: i64 = phase_conn
        .query_row("SELECT COUNT(*) FROM task_dependencies", [], |r| r.get(0))
        .unwrap();
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

    let dep_count: i64 = phase_conn
        .query_row("SELECT COUNT(*) FROM task_dependencies", [], |r| r.get(0))
        .unwrap();
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
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    // First step: all defaults
    assert_eq!(steps[0].0, "step"); // default name
    assert_eq!(steps[0].1, ""); // default command
    assert_eq!(steps[0].2, 0); // default order
    assert_eq!(steps[0].3, 1); // default required = true
    assert_eq!(steps[0].4, None); // default timeout = None

    // Second step: only name provided
    assert_eq!(steps[1].0, "lint");
    assert_eq!(steps[1].1, ""); // default command

    // Third step: explicit values
    assert_eq!(steps[2].0, "step"); // default name
    assert_eq!(steps[2].1, "cargo test");
    assert_eq!(steps[2].3, 0); // required = false
    assert_eq!(steps[2].4, Some(42)); // explicit timeout

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
