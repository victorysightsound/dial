use async_trait::async_trait;
use dial_core::prd;
use dial_core::provider::{Provider, ProviderRequest, ProviderResponse, TokenUsage};
use dial_core::Engine;
use serde_json::{json, Value as JsonValue};
use std::env;
use std::sync::Mutex;
use tempfile::TempDir;

static CWD_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// RAII guard that restores the CWD on drop (even if the test panics).
struct CwdGuard {
    original_dir: std::path::PathBuf,
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original_dir);
    }
}

async fn setup_engine() -> (Engine, TempDir, CwdGuard) {
    let original_dir =
        env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
    let tmp = TempDir::new().unwrap();
    env::set_current_dir(tmp.path()).unwrap();
    let engine = Engine::init("test", None, false).await.unwrap();
    (engine, tmp, CwdGuard { original_dir })
}

/// Seed spec_sections rows so phase 5 task inserts satisfy the FK constraint.
/// (The bundled SQLite compiles with SQLITE_DEFAULT_FOREIGN_KEYS=1, and
/// tasks.spec_section_id references spec_sections.id.)
fn seed_spec_sections() {
    let conn = dial_core::get_db(None).unwrap();
    for i in 1..=10 {
        conn.execute(
            "INSERT OR IGNORE INTO spec_sections (id, file_path, heading_path, level, content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                i as i64,
                "wizard_test.md",
                format!("Section {}", i),
                1,
                format!("Placeholder content for section {}", i)
            ],
        )
        .unwrap();
    }
}

// ---------------------------------------------------------------------------
// SequentialMockProvider — returns pre-defined responses in order
// ---------------------------------------------------------------------------

struct SequentialMockProvider {
    responses: Mutex<Vec<String>>,
}

impl SequentialMockProvider {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }

    fn remaining(&self) -> usize {
        self.responses.lock().unwrap().len()
    }
}

#[async_trait]
impl Provider for SequentialMockProvider {
    fn name(&self) -> &str {
        "sequential-mock"
    }

    async fn execute(&self, _request: ProviderRequest) -> dial_core::Result<ProviderResponse> {
        let mut responses = self.responses.lock().unwrap();
        let output = if responses.is_empty() {
            "{}".to_string()
        } else {
            responses.remove(0)
        };
        Ok(ProviderResponse {
            output,
            success: true,
            exit_code: Some(0),
            usage: Some(TokenUsage {
                tokens_in: 100,
                tokens_out: 200,
                cost_usd: Some(0.001),
            }),
            model: Some("mock-model".to_string()),
            duration_secs: Some(0.1),
        })
    }

    async fn is_available(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Phase response fixtures
// ---------------------------------------------------------------------------

fn phase_1_response() -> String {
    serde_json::to_string(&json!({
        "project_name": "WizardTestProject",
        "problem": "Need automated testing for the wizard flow",
        "target_users": "Developers building with DIAL",
        "success_criteria": ["All 9 phases complete", "Resume works", "Backward compat"]
    }))
    .unwrap()
}

fn phase_2_response() -> String {
    serde_json::to_string(&json!({
        "mvp_features": ["phase execution", "state persistence", "resume support"],
        "nice_to_have": ["progress reporting", "undo phase"],
        "out_of_scope": ["multi-user support"]
    }))
    .unwrap()
}

fn phase_3_response() -> String {
    serde_json::to_string(&json!({
        "languages": ["Rust"],
        "frameworks": ["tokio", "rusqlite"],
        "platform": "cross-platform CLI",
        "integrations": ["SQLite", "AI providers"],
        "constraints": ["must work offline", "single binary"]
    }))
    .unwrap()
}

fn phase_4_response() -> String {
    serde_json::to_string(&json!({
        "gaps": [
            {"area": "testing", "description": "Integration tests missing for wizard flow"},
            {"area": "error handling", "description": "Need retry on provider timeout"}
        ],
        "recommendations": ["Add integration test suite", "Implement timeout retry logic"],
        "section_ratings": [
            {"section": "vision", "rating": "SPECIFIC", "issues": []},
            {"section": "functionality", "rating": "NEEDS_DETAIL", "issues": ["mvp_features lacks acceptance criteria"]},
            {"section": "technical", "rating": "SPECIFIC", "issues": []}
        ],
        "rewritten_sections": []
    }))
    .unwrap()
}

fn phase_5_response() -> String {
    serde_json::to_string(&json!({
        "sections": [
            {"title": "Overview", "content": "Project overview for the wizard test project"},
            {"title": "Architecture", "content": "The system uses a phase-based wizard architecture"},
            {"title": "Implementation", "content": "Implementation details for each wizard phase"}
        ],
        "terminology": [
            {"term": "DIAL", "definition": "Deterministic Iterative Agent Loop", "category": "acronym"},
            {"term": "PRD", "definition": "Product Requirements Document", "category": "acronym"}
        ]
    }))
    .unwrap()
}

fn phase_6_response() -> String {
    serde_json::to_string(&json!({
        "tasks": [
            {
                "description": "Set up project scaffolding",
                "priority": 1,
                "spec_section": "1",
                "depends_on": [],
                "rationale": "Foundation first"
            },
            {
                "description": "Implement wizard phase engine",
                "priority": 2,
                "spec_section": "2",
                "depends_on": [0],
                "rationale": "Core logic depends on scaffolding"
            },
            {
                "description": "Add integration tests",
                "priority": 3,
                "spec_section": "3",
                "depends_on": [1],
                "rationale": "Tests after implementation"
            }
        ],
        "removed": [
            {"original": "Implement: Overview", "reason": "Too vague, replaced with specific tasks"}
        ],
        "added": [
            {"description": "Add integration tests", "reason": "Testing coverage needed"}
        ]
    }))
    .unwrap()
}

fn phase_7_response() -> String {
    serde_json::to_string(&json!({
        "build_cmd": "cargo build --release",
        "test_cmd": "cargo test --all",
        "pipeline_steps": [
            {"name": "lint", "command": "cargo clippy", "order": 1, "required": true, "timeout": 60},
            {"name": "build", "command": "cargo build", "order": 2, "required": true, "timeout": 300},
            {"name": "test", "command": "cargo test", "order": 3, "required": true, "timeout": 120}
        ],
        "build_timeout": 300,
        "test_timeout": 120,
        "rationale": "Standard Rust pipeline with clippy lint step"
    }))
    .unwrap()
}

fn phase_8_response() -> String {
    serde_json::to_string(&json!({
        "recommended_mode": "review_every",
        "review_interval": 3,
        "ai_cli": "claude",
        "subagent_timeout": 1800,
        "rationale": "Medium complexity project, review every 3 tasks"
    }))
    .unwrap()
}

/// All 8 provider responses (phases 1-8). Phase 9 has no provider call.
fn all_provider_responses() -> Vec<String> {
    vec![
        phase_1_response(),
        phase_2_response(),
        phase_3_response(),
        phase_4_response(),
        phase_5_response(),
        phase_6_response(),
        phase_7_response(),
        phase_8_response(),
    ]
}

/// Build gathered_info as if phases 1..=n were completed.
fn gathered_info_through_phase(n: i32) -> JsonValue {
    let mut info = json!({});
    if n >= 1 {
        info["vision"] = json!({
            "project_name": "WizardTestProject",
            "problem": "Need automated testing for the wizard flow",
            "target_users": "Developers building with DIAL",
            "success_criteria": ["All 9 phases complete", "Resume works", "Backward compat"]
        });
    }
    if n >= 2 {
        info["functionality"] = json!({
            "mvp_features": ["phase execution", "state persistence", "resume support"],
            "nice_to_have": ["progress reporting", "undo phase"],
            "out_of_scope": ["multi-user support"]
        });
    }
    if n >= 3 {
        info["technical"] = json!({
            "languages": ["Rust"],
            "frameworks": ["tokio", "rusqlite"],
            "platform": "cross-platform CLI",
            "integrations": ["SQLite", "AI providers"],
            "constraints": ["must work offline", "single binary"]
        });
    }
    if n >= 4 {
        info["gap_analysis"] = json!({
            "gaps": [
                {"area": "testing", "description": "Integration tests missing for wizard flow"},
                {"area": "error handling", "description": "Need retry on provider timeout"}
            ],
            "recommendations": ["Add integration test suite", "Implement timeout retry logic"],
            "section_ratings": [
                {"section": "vision", "rating": "SPECIFIC", "issues": []},
                {"section": "functionality", "rating": "NEEDS_DETAIL", "issues": ["mvp_features lacks acceptance criteria"]},
                {"section": "technical", "rating": "SPECIFIC", "issues": []}
            ],
            "rewritten_sections": []
        });
    }
    if n >= 5 {
        info["generate"] = json!({
            "sections": [
                {"title": "Overview", "content": "Project overview for the wizard test project"},
                {"title": "Architecture", "content": "The system uses a phase-based wizard architecture"},
                {"title": "Implementation", "content": "Implementation details for each wizard phase"}
            ],
            "terminology": [
                {"term": "DIAL", "definition": "Deterministic Iterative Agent Loop", "category": "acronym"},
                {"term": "PRD", "definition": "Product Requirements Document", "category": "acronym"}
            ]
        });
    }
    if n >= 6 {
        info["task_review"] = json!({
            "tasks": [
                {"description": "Set up project scaffolding", "priority": 1, "spec_section": "1", "depends_on": [], "rationale": "Foundation first"},
                {"description": "Implement wizard phase engine", "priority": 2, "spec_section": "2", "depends_on": [0], "rationale": "Core logic depends on scaffolding"},
                {"description": "Add integration tests", "priority": 3, "spec_section": "3", "depends_on": [1], "rationale": "Tests after implementation"}
            ],
            "removed": [{"original": "Implement: Overview", "reason": "Too vague"}],
            "added": [{"description": "Add integration tests", "reason": "Testing coverage needed"}]
        });
    }
    if n >= 7 {
        info["build_&_test_config"] = json!({
            "build_cmd": "cargo build --release",
            "test_cmd": "cargo test --all",
            "pipeline_steps": [
                {"name": "lint", "command": "cargo clippy", "order": 1, "required": true, "timeout": 60},
                {"name": "build", "command": "cargo build", "order": 2, "required": true, "timeout": 300},
                {"name": "test", "command": "cargo test", "order": 3, "required": true, "timeout": 120}
            ],
            "build_timeout": 300,
            "test_timeout": 120,
            "rationale": "Standard Rust pipeline"
        });
    }
    if n >= 8 {
        info["iteration_mode"] = json!({
            "recommended_mode": "review_every",
            "review_interval": 3,
            "ai_cli": "claude",
            "subagent_timeout": 1800,
            "rationale": "Medium complexity project, review every 3 tasks"
        });
    }
    info
}

/// Set up the DIAL database state as if phases through `n` completed.
///
/// Phase 5 inserts PRD sections and DIAL tasks.
/// Phase 6 replaces tasks with reviewed versions.
/// Phase 7 writes config values.
/// Phase 8 writes iteration mode config.
fn setup_db_through_phase(prd_conn: &rusqlite::Connection, n: i32) {
    if n >= 5 {
        // Phase 5 inserts PRD sections
        for (i, (title, content)) in [
            ("Overview", "Project overview for the wizard test project"),
            (
                "Architecture",
                "The system uses a phase-based wizard architecture",
            ),
            (
                "Implementation",
                "Implementation details for each wizard phase",
            ),
        ]
        .iter()
        .enumerate()
        {
            let section_id = format!("{}", i + 1);
            let word_count = content.split_whitespace().count() as i32;
            prd::prd_insert_section(
                prd_conn,
                &section_id,
                title,
                None,
                1,
                i as i32,
                content,
                word_count,
            )
            .unwrap();
        }

        // Phase 5 inserts DIAL tasks (spec_section_id needs matching spec_sections rows)
        seed_spec_sections();
        let phase_conn = dial_core::get_db(None).unwrap();
        for (i, title) in ["Overview", "Architecture", "Implementation"]
            .iter()
            .enumerate()
        {
            let desc = format!("Implement: {}", title);
            let priority = (i + 1) as i32;
            phase_conn
                .execute(
                    "INSERT INTO tasks (description, status, priority, spec_section_id)
                     VALUES (?1, 'pending', ?2, ?3)",
                    rusqlite::params![desc, priority, (i + 1) as i64],
                )
                .unwrap();
        }
    }

    if n >= 6 {
        // Phase 6 replaces tasks with reviewed versions
        let phase_conn = dial_core::get_db(None).unwrap();
        phase_conn
            .execute(
                "DELETE FROM task_dependencies WHERE task_id IN (
                    SELECT id FROM tasks WHERE status IN ('pending', 'in_progress')
                 ) OR depends_on_id IN (
                    SELECT id FROM tasks WHERE status IN ('pending', 'in_progress')
                 )",
                [],
            )
            .unwrap();
        phase_conn
            .execute(
                "DELETE FROM tasks WHERE status IN ('pending', 'in_progress')",
                [],
            )
            .unwrap();

        // Insert reviewed tasks (prd_section_id is TEXT, no FK constraint)
        let mut ids: Vec<i64> = Vec::new();
        for (desc, priority, section) in [
            ("Set up project scaffolding", 1, "1"),
            ("Implement wizard phase engine", 2, "2"),
            ("Add integration tests", 3, "3"),
        ] {
            phase_conn
                .execute(
                    "INSERT INTO tasks (description, status, priority, prd_section_id)
                     VALUES (?1, 'pending', ?2, ?3)",
                    rusqlite::params![desc, priority, section],
                )
                .unwrap();
            ids.push(phase_conn.last_insert_rowid());
        }
        // Task 1 depends on task 0, task 2 depends on task 1
        phase_conn
            .execute(
                "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id)
                 VALUES (?1, ?2)",
                rusqlite::params![ids[1], ids[0]],
            )
            .unwrap();
        phase_conn
            .execute(
                "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id)
                 VALUES (?1, ?2)",
                rusqlite::params![ids[2], ids[1]],
            )
            .unwrap();
    }

    if n >= 7 {
        dial_core::config::config_set("build_cmd", "cargo build --release").unwrap();
        dial_core::config::config_set("test_cmd", "cargo test --all").unwrap();
        dial_core::config::config_set("build_timeout", "300").unwrap();
        dial_core::config::config_set("test_timeout", "120").unwrap();

        let phase_conn = dial_core::get_db(None).unwrap();
        phase_conn
            .execute("DELETE FROM validation_steps", [])
            .unwrap();
        for (name, command, order, required, timeout) in [
            ("lint", "cargo clippy", 1, true, 60),
            ("build", "cargo build", 2, true, 300),
            ("test", "cargo test", 3, true, 120),
        ] {
            phase_conn
                .execute(
                    "INSERT INTO validation_steps (name, command, sort_order, required, timeout_secs)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![
                        name,
                        command,
                        order,
                        if required { 1 } else { 0 },
                        timeout
                    ],
                )
                .unwrap();
        }
    }

    if n >= 8 {
        dial_core::config::config_set("iteration_mode", "review_every:3").unwrap();
        dial_core::config::config_set("ai_cli", "claude").unwrap();
        dial_core::config::config_set("subagent_timeout", "1800").unwrap();
    }
}

/// Save a pre-built wizard state with phases 1..=n completed.
fn save_state_through_phase(prd_conn: &rusqlite::Connection, n: i32) {
    let mut state = prd::wizard::WizardState::new("spec");
    state.gathered_info = gathered_info_through_phase(n);
    for phase_num in 1..=n {
        state.completed_phases.push(phase_num);
    }
    if n < 9 {
        state.current_phase =
            prd::wizard::WizardPhase::from_i32(n + 1).unwrap_or(prd::wizard::WizardPhase::Launch);
    } else {
        state.current_phase = prd::wizard::WizardPhase::Launch;
    }
    prd::wizard::save_wizard_state(prd_conn, &state).unwrap();
}

/// Provider responses needed for phases (from+1)..=8 (phase 9 has no provider call).
fn responses_from_phase(from: i32) -> Vec<String> {
    let all = all_provider_responses();
    let start = from as usize;
    if start >= all.len() {
        vec![]
    } else {
        all[start..].to_vec()
    }
}

// ===========================================================================
// Test: Full 9-phase wizard with mock provider
// ===========================================================================

#[tokio::test]
async fn test_full_wizard_all_9_phases() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    // Seed spec_sections so phase 5 task inserts satisfy FK
    seed_spec_sections();

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    let provider = SequentialMockProvider::new(all_provider_responses());

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, false, true)
        .await
        .unwrap();

    // All provider responses consumed (8 calls for phases 1-8)
    assert_eq!(
        provider.remaining(),
        0,
        "All provider responses should be consumed"
    );

    // Phase 5 generates sections and tasks
    assert_eq!(result.sections_generated, 3);
    assert_eq!(result.tasks_generated, 3);

    // Phase 6 task review
    assert_eq!(result.tasks_added, 1);
    assert_eq!(result.tasks_removed, 1);
    assert_eq!(result.tasks_kept, 2); // 3 total - 1 added = 2 kept

    // Phase 7 build/test config
    assert_eq!(result.build_cmd, "cargo build --release");
    assert_eq!(result.test_cmd, "cargo test --all");
    assert_eq!(result.pipeline_steps, 3);

    // Phase 8 iteration mode
    assert_eq!(result.iteration_mode, "review_every:3");

    // Phase 9 launch
    assert_eq!(result.project_name, "WizardTestProject");
    assert!(result.task_count > 0);

    // Verify wizard state is fully complete
    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(
            state.completed_phases.contains(&phase_num),
            "Phase {} should be completed",
            phase_num
        );
    }

    // Verify gathered_info has all phases
    assert!(state.gathered_info.get("vision").is_some());
    assert!(state.gathered_info.get("functionality").is_some());
    assert!(state.gathered_info.get("technical").is_some());
    assert!(state.gathered_info.get("gap_analysis").is_some());
    assert!(state.gathered_info.get("generate").is_some());
    assert!(state.gathered_info.get("task_review").is_some());
    assert!(state.gathered_info.get("build_&_test_config").is_some());
    assert!(state.gathered_info.get("iteration_mode").is_some());
    assert!(state.gathered_info.get("launch").is_some());
    assert_eq!(
        state.gathered_info["launch"]["launch_ready"].as_bool(),
        Some(true)
    );

    // Verify PRD sections exist
    let sections = prd::prd_list_sections(&prd_conn).unwrap();
    assert_eq!(sections.len(), 3);

    // Verify config values from phase 7
    let build_cmd = dial_core::config::config_get("build_cmd").unwrap();
    assert_eq!(build_cmd, Some("cargo build --release".to_string()));
    let test_cmd = dial_core::config::config_get("test_cmd").unwrap();
    assert_eq!(test_cmd, Some("cargo test --all".to_string()));

    // Verify config values from phase 8
    let mode = dial_core::config::config_get("iteration_mode").unwrap();
    assert_eq!(mode, Some("review_every:3".to_string()));
    let ai_cli = dial_core::config::config_get("ai_cli").unwrap();
    assert_eq!(ai_cli, Some("claude".to_string()));

    // Verify DIAL tasks exist (from phase 6 review)
    let phase_conn = dial_core::get_db(None).unwrap();
    let task_count: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(task_count, 3);

    // Verify task dependencies from phase 6
    let dep_count: i64 = phase_conn
        .query_row("SELECT COUNT(*) FROM task_dependencies", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(dep_count, 2); // task 1->0, task 2->1

    // Verify validation steps from phase 7
    let step_count: i64 = phase_conn
        .query_row("SELECT COUNT(*) FROM validation_steps", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(step_count, 3);
}

// ===========================================================================
// Test: full=false backward compatibility (phases 1-5 only)
// ===========================================================================

#[tokio::test]
async fn test_wizard_full_false_only_runs_phases_1_to_5() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    seed_spec_sections();

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    let responses: Vec<String> = all_provider_responses().into_iter().take(5).collect();
    let provider = SequentialMockProvider::new(responses);

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, false, false)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);

    // Phase 5 generates sections and tasks
    assert_eq!(result.sections_generated, 3);
    assert_eq!(result.tasks_generated, 3);

    // Phase 6-9 results should be default (not executed)
    assert_eq!(result.tasks_kept, 0);
    assert_eq!(result.tasks_added, 0);
    assert_eq!(result.tasks_removed, 0);
    assert_eq!(result.build_cmd, "");
    assert_eq!(result.test_cmd, "");
    assert_eq!(result.pipeline_steps, 0);
    assert_eq!(result.iteration_mode, "");
    assert_eq!(result.project_name, "");
    assert_eq!(result.task_count, 0);

    // Verify wizard state has only phases 1-5 completed
    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=5 {
        assert!(
            state.completed_phases.contains(&phase_num),
            "Phase {} should be completed",
            phase_num
        );
    }
    for phase_num in 6..=9 {
        assert!(
            !state.completed_phases.contains(&phase_num),
            "Phase {} should NOT be completed in full=false mode",
            phase_num
        );
    }

    // Verify no phase 8 config was written
    let mode = dial_core::config::config_get("iteration_mode").unwrap();
    assert!(
        mode.is_none(),
        "iteration_mode should not be set in full=false mode"
    );

    // Verify PRD sections exist (from phase 5)
    let sections = prd::prd_list_sections(&prd_conn).unwrap();
    assert_eq!(sections.len(), 3);
}

// ===========================================================================
// Test: Resume from phase 2 (phase 1 already complete)
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_from_phase_2() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    seed_spec_sections();
    let prd_conn = prd::get_or_init_prd_db().unwrap();

    save_state_through_phase(&prd_conn, 1);

    let provider = SequentialMockProvider::new(responses_from_phase(1));

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);
    assert_eq!(result.sections_generated, 3);
    assert_eq!(result.project_name, "WizardTestProject");

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(
            state.completed_phases.contains(&phase_num),
            "Phase {} should be completed after resume",
            phase_num
        );
    }
}

// ===========================================================================
// Test: Resume from phase 3 (phases 1-2 already complete)
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_from_phase_3() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    seed_spec_sections();
    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 2);

    let provider = SequentialMockProvider::new(responses_from_phase(2));

    let _result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(
            state.completed_phases.contains(&phase_num),
            "Phase {} should be completed after resume from 3",
            phase_num
        );
    }

    // Verify phase 1-2 data preserved from pre-populated state
    assert_eq!(
        state.gathered_info["vision"]["project_name"].as_str(),
        Some("WizardTestProject")
    );
    assert!(state.gathered_info["functionality"]["mvp_features"]
        .as_array()
        .is_some());
}

// ===========================================================================
// Test: Resume from phase 4 (phases 1-3 already complete)
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_from_phase_4() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    seed_spec_sections();
    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 3);

    let provider = SequentialMockProvider::new(responses_from_phase(3));

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);
    assert_eq!(result.sections_generated, 3);

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(state.completed_phases.contains(&phase_num));
    }
}

// ===========================================================================
// Test: Resume from phase 5 (phases 1-4 already complete)
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_from_phase_5() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    seed_spec_sections();
    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 4);

    let provider = SequentialMockProvider::new(responses_from_phase(4));

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);
    assert_eq!(result.sections_generated, 3);
    assert_eq!(result.tasks_generated, 3);

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(state.completed_phases.contains(&phase_num));
    }
}

// ===========================================================================
// Test: Resume from phase 6 (phases 1-5 already complete)
// This is the boundary between full=false and full=true.
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_from_phase_6() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();

    save_state_through_phase(&prd_conn, 5);
    setup_db_through_phase(&prd_conn, 5);

    let provider = SequentialMockProvider::new(responses_from_phase(5));

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);

    // Phases 1-5 were pre-populated, so sections/tasks_generated should be 0
    assert_eq!(result.sections_generated, 0);
    assert_eq!(result.tasks_generated, 0);

    // Phase 6 reviewed tasks
    assert!(result.tasks_kept > 0 || result.tasks_added > 0);

    // Phase 7
    assert_eq!(result.build_cmd, "cargo build --release");
    assert_eq!(result.test_cmd, "cargo test --all");
    assert_eq!(result.pipeline_steps, 3);

    // Phase 8
    assert_eq!(result.iteration_mode, "review_every:3");

    // Phase 9
    assert_eq!(result.project_name, "WizardTestProject");

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(
            state.completed_phases.contains(&phase_num),
            "Phase {} should be completed after resume from 6",
            phase_num
        );
    }
}

// ===========================================================================
// Test: Resume from phase 7 (phases 1-6 already complete)
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_from_phase_7() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 6);
    setup_db_through_phase(&prd_conn, 6);

    let provider = SequentialMockProvider::new(responses_from_phase(6));

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);
    assert_eq!(result.build_cmd, "cargo build --release");
    assert_eq!(result.test_cmd, "cargo test --all");
    assert_eq!(result.iteration_mode, "review_every:3");
    assert_eq!(result.project_name, "WizardTestProject");

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(state.completed_phases.contains(&phase_num));
    }
}

// ===========================================================================
// Test: Resume from phase 8 (phases 1-7 already complete)
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_from_phase_8() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 7);
    setup_db_through_phase(&prd_conn, 7);

    let provider = SequentialMockProvider::new(responses_from_phase(7));

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);
    assert_eq!(result.iteration_mode, "review_every:3");
    assert_eq!(result.project_name, "WizardTestProject");

    // Phase 7 results are 0/empty because phase 7 was already complete
    assert_eq!(result.build_cmd, "");
    assert_eq!(result.test_cmd, "");

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(state.completed_phases.contains(&phase_num));
    }
}

// ===========================================================================
// Test: Resume from phase 9 (phases 1-8 already complete)
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_from_phase_9() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 8);
    setup_db_through_phase(&prd_conn, 8);

    // No provider calls needed (phase 9 doesn't call provider)
    let provider = SequentialMockProvider::new(vec![]);

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);
    assert_eq!(result.project_name, "WizardTestProject");
    assert!(result.task_count > 0);

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(
            state.completed_phases.contains(&phase_num),
            "Phase {} should be completed after resume from 9",
            phase_num
        );
    }
    assert_eq!(
        state.gathered_info["launch"]["launch_ready"].as_bool(),
        Some(true)
    );
}

// ===========================================================================
// Test: Resume when all phases already complete — should skip everything
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_all_complete_skips_everything() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    setup_db_through_phase(&prd_conn, 8);

    // Save state with all 9 phases complete (including launch data)
    let mut state = prd::wizard::WizardState::new("spec");
    state.gathered_info = gathered_info_through_phase(8);
    state.gathered_info["launch"] = json!({
        "launch_ready": true,
        "project_name": "WizardTestProject",
        "task_count": 3,
    });
    for phase_num in 1..=9 {
        state.completed_phases.push(phase_num);
    }
    state.current_phase = prd::wizard::WizardPhase::Launch;
    prd::wizard::save_wizard_state(&prd_conn, &state).unwrap();

    // No provider calls should happen
    let provider = SequentialMockProvider::new(vec![]);

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);

    // All result fields are default (no phases executed)
    assert_eq!(result.sections_generated, 0);
    assert_eq!(result.tasks_generated, 0);
    assert_eq!(result.tasks_kept, 0);
    assert_eq!(result.build_cmd, "");
    assert_eq!(result.test_cmd, "");
    assert_eq!(result.iteration_mode, "");
    assert_eq!(result.project_name, "");
}

// ===========================================================================
// Test: Resume with no saved state behaves like fresh start
// ===========================================================================

#[tokio::test]
async fn test_wizard_resume_no_saved_state_fresh_start() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    seed_spec_sections();
    let prd_conn = prd::get_or_init_prd_db().unwrap();

    let provider = SequentialMockProvider::new(all_provider_responses());

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, true)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);
    assert_eq!(result.sections_generated, 3);
    assert_eq!(result.project_name, "WizardTestProject");

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=9 {
        assert!(state.completed_phases.contains(&phase_num));
    }
}

// ===========================================================================
// Test: full=false resume skips phases 6-9 even if state has partial completion
// ===========================================================================

#[tokio::test]
async fn test_wizard_full_false_resume_stops_at_phase_5() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    seed_spec_sections();
    let prd_conn = prd::get_or_init_prd_db().unwrap();

    save_state_through_phase(&prd_conn, 3);

    let responses = vec![phase_4_response(), phase_5_response()];
    let provider = SequentialMockProvider::new(responses);

    let result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, false)
        .await
        .unwrap();

    assert_eq!(provider.remaining(), 0);
    assert_eq!(result.sections_generated, 3);

    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    for phase_num in 1..=5 {
        assert!(state.completed_phases.contains(&phase_num));
    }
    for phase_num in 6..=9 {
        assert!(!state.completed_phases.contains(&phase_num));
    }
}

// ===========================================================================
// Test: Invalid template returns error
// ===========================================================================

#[tokio::test]
async fn test_wizard_invalid_template_returns_error() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    let provider = SequentialMockProvider::new(vec![]);

    let result = prd::wizard::run_wizard(
        &provider,
        &prd_conn,
        "nonexistent_template",
        None,
        false,
        true,
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("nonexistent_template"),
        "Error should mention the invalid template name: {}",
        err
    );
}

// ===========================================================================
// Test: Provider failure mid-wizard propagates error
// ===========================================================================

#[tokio::test]
async fn test_wizard_provider_failure_propagates_error() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();

    struct FailOnSecondCallProvider {
        call_count: Mutex<usize>,
    }

    #[async_trait]
    impl Provider for FailOnSecondCallProvider {
        fn name(&self) -> &str {
            "fail-on-second"
        }
        async fn execute(
            &self,
            _request: ProviderRequest,
        ) -> dial_core::Result<ProviderResponse> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            if *count == 1 {
                Ok(ProviderResponse {
                    output: phase_1_response(),
                    success: true,
                    exit_code: Some(0),
                    usage: None,
                    model: None,
                    duration_secs: None,
                })
            } else {
                Ok(ProviderResponse {
                    output: "Provider error: rate limited".to_string(),
                    success: false,
                    exit_code: Some(1),
                    usage: None,
                    model: None,
                    duration_secs: None,
                })
            }
        }
        async fn is_available(&self) -> bool {
            true
        }
    }

    let provider = FailOnSecondCallProvider {
        call_count: Mutex::new(0),
    };

    let result =
        prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, false, true).await;

    assert!(result.is_err());

    // Phase 1 should be completed, phase 2 should not
    let state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    assert!(state.completed_phases.contains(&1));
    assert!(!state.completed_phases.contains(&2));
}

// ===========================================================================
// Test: Phase 6 — task replacement with dependency relationships
// ===========================================================================

#[tokio::test]
async fn test_phase_6_task_replacement_with_dependencies() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 5);
    setup_db_through_phase(&prd_conn, 5);

    // Pre-condition: 3 original tasks from phase 5 exist
    let phase_conn = dial_core::get_db(None).unwrap();
    let original_count: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original_count, 3, "Should have 3 original tasks from phase 5");

    // Verify original tasks have "Implement: " prefix
    let old_count: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE description LIKE 'Implement: %'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_count, 3, "All 3 original tasks should have 'Implement: ' prefix");

    // Load state and run phase 6 directly
    let mut state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    let provider = SequentialMockProvider::new(vec![phase_6_response()]);

    let (kept, added, removed, _sizing) =
        prd::wizard::run_wizard_phase_6(&provider, &prd_conn, &mut state)
            .await
            .unwrap();

    assert_eq!(kept, 2);
    assert_eq!(added, 1);
    assert_eq!(removed, 1);

    // Original "Implement: ..." tasks should be gone
    let phase_conn = dial_core::get_db(None).unwrap();
    let old_remaining: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE description LIKE 'Implement: %'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_remaining, 0, "Original phase 5 tasks should be deleted");

    // 3 reviewed tasks should exist
    let mut stmt = phase_conn
        .prepare(
            "SELECT id, description, priority, prd_section_id
             FROM tasks WHERE status = 'pending' ORDER BY priority, id",
        )
        .unwrap();
    let tasks: Vec<(i64, String, i32, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    assert_eq!(tasks.len(), 3, "Should have 3 reviewed tasks");

    // Verify descriptions, priorities, and prd_section_ids
    assert_eq!(tasks[0].1, "Set up project scaffolding");
    assert_eq!(tasks[0].2, 1);
    assert_eq!(tasks[0].3, Some("1".to_string()));

    assert_eq!(tasks[1].1, "Implement wizard phase engine");
    assert_eq!(tasks[1].2, 2);
    assert_eq!(tasks[1].3, Some("2".to_string()));

    assert_eq!(tasks[2].1, "Add integration tests");
    assert_eq!(tasks[2].2, 3);
    assert_eq!(tasks[2].3, Some("3".to_string()));

    // Verify dependency count
    let dep_count: i64 = phase_conn
        .query_row("SELECT COUNT(*) FROM task_dependencies", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(dep_count, 2, "Should have 2 dependency relationships");

    let scaffolding_id = tasks[0].0;
    let engine_id = tasks[1].0;
    let tests_id = tasks[2].0;

    // "Implement wizard phase engine" depends on "Set up project scaffolding"
    let dep_exists: bool = phase_conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM task_dependencies
             WHERE task_id = ?1 AND depends_on_id = ?2",
            rusqlite::params![engine_id, scaffolding_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(dep_exists, "Engine task should depend on scaffolding task");

    // "Add integration tests" depends on "Implement wizard phase engine"
    let dep_exists: bool = phase_conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM task_dependencies
             WHERE task_id = ?1 AND depends_on_id = ?2",
            rusqlite::params![tests_id, engine_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(dep_exists, "Tests task should depend on engine task");

    // "Set up project scaffolding" has no dependencies
    let scaffolding_deps: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM task_dependencies WHERE task_id = ?1",
            rusqlite::params![scaffolding_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        scaffolding_deps, 0,
        "Scaffolding task should have no dependencies"
    );

    // Verify auto_run's dependency-aware query finds ONLY scaffolding as eligible
    // (same query from orchestrator.rs lines 330-338)
    let eligible_id: i64 = phase_conn
        .query_row(
            "SELECT id FROM tasks WHERE status = 'pending'
             AND id NOT IN (
                 SELECT td.task_id FROM task_dependencies td
                 INNER JOIN tasks dep ON dep.id = td.depends_on_id
                 WHERE dep.status != 'completed'
             )
             ORDER BY priority, id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        eligible_id, scaffolding_id,
        "Only scaffolding task should be eligible (no unmet dependencies)"
    );

    // Verify total eligible count is exactly 1
    let eligible_count: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'pending'
             AND id NOT IN (
                 SELECT td.task_id FROM task_dependencies td
                 INNER JOIN tasks dep ON dep.id = td.depends_on_id
                 WHERE dep.status != 'completed'
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        eligible_count, 1,
        "Exactly 1 task should be eligible when dependencies block the others"
    );
}

// ===========================================================================
// Test: Phase 6 — task splitting via sizing analysis
// ===========================================================================

/// Response fixture where the AI splits a large task into sub-tasks,
/// rewrites a vague task, and merges two small tasks.
fn phase_6_response_with_sizing() -> String {
    serde_json::to_string(&json!({
        "tasks": [
            {
                "description": "Create users table with columns: id, email, password_hash, created_at",
                "priority": 1,
                "spec_section": "1",
                "depends_on": [],
                "rationale": "Database schema first",
                "size": "S"
            },
            {
                "description": "Add bcrypt password hashing to User model with cost factor 12",
                "priority": 2,
                "spec_section": "1",
                "depends_on": [0],
                "rationale": "Auth depends on user table",
                "size": "M"
            },
            {
                "description": "Add POST /login endpoint returning JWT with 24h expiry",
                "priority": 3,
                "spec_section": "1",
                "depends_on": [0, 1],
                "rationale": "Login needs both table and hashing",
                "size": "M"
            },
            {
                "description": "Set up project config files (.gitignore, .env.example, CI pipeline)",
                "priority": 4,
                "spec_section": "2",
                "depends_on": [],
                "rationale": "Config can happen in parallel",
                "size": "S"
            }
        ],
        "removed": [
            {"original": "Implement: Overview", "reason": "Too vague, replaced with specific tasks"}
        ],
        "added": [
            {"description": "Add POST /login endpoint returning JWT with 24h expiry", "reason": "Login was missing"}
        ],
        "splits": [
            {
                "original": "Implement: Architecture",
                "into": [
                    "Create users table with columns: id, email, password_hash, created_at",
                    "Add bcrypt password hashing to User model with cost factor 12",
                    "Add POST /login endpoint returning JWT with 24h expiry"
                ],
                "reason": "Original task touched >3 files and implemented multiple features"
            }
        ],
        "rewrites": [
            {
                "original": "Implement: Implementation",
                "rewritten": "Set up project config files (.gitignore, .env.example, CI pipeline)",
                "reason": "Original was vague with no concrete deliverables"
            }
        ],
        "merges": [],
        "sizing_summary": {
            "S": 2, "M": 2, "L": 0, "XL": 0,
            "total_splits": 1, "total_rewrites": 1, "total_merges": 0
        }
    }))
    .unwrap()
}

#[tokio::test]
async fn test_phase_6_task_splitting_via_sizing() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 5);
    setup_db_through_phase(&prd_conn, 5);

    // Pre-condition: 3 original tasks from phase 5 exist
    let phase_conn = dial_core::get_db(None).unwrap();
    let original_count: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original_count, 3, "Should have 3 original tasks from phase 5");

    // Load state and run phase 6 with sizing response
    let mut state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    let provider = SequentialMockProvider::new(vec![phase_6_response_with_sizing()]);

    let (kept, added, removed, sizing) =
        prd::wizard::run_wizard_phase_6(&provider, &prd_conn, &mut state)
            .await
            .unwrap();

    assert_eq!(kept, 3);
    assert_eq!(added, 1);
    assert_eq!(removed, 1);

    // Verify sizing summary
    assert_eq!(sizing.small, 2);
    assert_eq!(sizing.medium, 2);
    assert_eq!(sizing.large, 0);
    assert_eq!(sizing.xl, 0);
    assert_eq!(sizing.total_splits, 1);
    assert_eq!(sizing.total_rewrites, 1);
    assert_eq!(sizing.total_merges, 0);

    // Verify tasks were replaced
    let phase_conn = dial_core::get_db(None).unwrap();
    let new_count: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(new_count, 4, "Should have 4 tasks after split");

    // Verify split sub-tasks exist with correct descriptions
    let users_table: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE description LIKE '%users table%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(users_table, 1, "Split sub-task 'users table' should exist");

    let bcrypt: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE description LIKE '%bcrypt%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bcrypt, 1, "Split sub-task 'bcrypt' should exist");

    let login: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE description LIKE '%login%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(login, 1, "Split sub-task 'login' should exist");

    // Verify dependencies: login task depends on both users_table and bcrypt
    let dep_count: i64 = phase_conn
        .query_row("SELECT COUNT(*) FROM task_dependencies", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(dep_count, 3, "Should have 3 dependency relationships");

    // Verify rewritten task exists
    let config_task: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE description LIKE '%config files%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(config_task, 1, "Rewritten task should exist with concrete description");

    // Original vague tasks should be gone
    let old_remaining: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE description LIKE 'Implement: %'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_remaining, 0, "Original vague tasks should all be replaced");
}

// ===========================================================================
// Test: Phase 7 — config values and pipeline step writing
// ===========================================================================

#[tokio::test]
async fn test_phase_7_config_and_pipeline_writing() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 6);
    setup_db_through_phase(&prd_conn, 6);

    // Insert a pre-existing validation step to verify it gets cleared
    let phase_conn = dial_core::get_db(None).unwrap();
    phase_conn
        .execute(
            "INSERT INTO validation_steps (name, command, sort_order, required, timeout_secs)
             VALUES ('old_step', 'echo old', 0, 1, 10)",
            [],
        )
        .unwrap();
    let pre_count: i64 = phase_conn
        .query_row("SELECT COUNT(*) FROM validation_steps", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(pre_count, 1, "Should have 1 pre-existing validation step");

    // Load state and run phase 7 directly
    let mut state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    let provider = SequentialMockProvider::new(vec![phase_7_response()]);

    let (build_cmd, test_cmd, steps_count) =
        prd::wizard::run_wizard_phase_7(&provider, &prd_conn, &mut state)
            .await
            .unwrap();

    assert_eq!(build_cmd, "cargo build --release");
    assert_eq!(test_cmd, "cargo test --all");
    assert_eq!(steps_count, 3);

    // Verify all 4 config values
    assert_eq!(
        dial_core::config::config_get("build_cmd").unwrap(),
        Some("cargo build --release".to_string())
    );
    assert_eq!(
        dial_core::config::config_get("test_cmd").unwrap(),
        Some("cargo test --all".to_string())
    );
    assert_eq!(
        dial_core::config::config_get("build_timeout").unwrap(),
        Some("300".to_string())
    );
    assert_eq!(
        dial_core::config::config_get("test_timeout").unwrap(),
        Some("120".to_string())
    );

    // Verify pre-existing step was cleared and 3 new steps inserted
    let phase_conn = dial_core::get_db(None).unwrap();
    let step_count: i64 = phase_conn
        .query_row("SELECT COUNT(*) FROM validation_steps", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        step_count, 3,
        "Old step should be cleared, 3 new steps inserted"
    );

    // No trace of the old step
    let old_step: i64 = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM validation_steps WHERE name = 'old_step'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old_step, 0, "Pre-existing 'old_step' should be cleared");

    // Verify each step's properties in order
    let mut stmt = phase_conn
        .prepare(
            "SELECT name, command, sort_order, required, timeout_secs
             FROM validation_steps ORDER BY sort_order",
        )
        .unwrap();
    let steps: Vec<(String, String, i32, bool, Option<i64>)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, i32>(3)? != 0,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    assert_eq!(
        steps[0],
        (
            "lint".to_string(),
            "cargo clippy".to_string(),
            1,
            true,
            Some(60)
        )
    );
    assert_eq!(
        steps[1],
        (
            "build".to_string(),
            "cargo build".to_string(),
            2,
            true,
            Some(300)
        )
    );
    assert_eq!(
        steps[2],
        (
            "test".to_string(),
            "cargo test".to_string(),
            3,
            true,
            Some(120)
        )
    );

    // Verify wizard state was updated
    let updated_state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    assert!(updated_state.completed_phases.contains(&7));
    assert!(updated_state.gathered_info.get("build_&_test_config").is_some());
}

// ===========================================================================
// Test: Phase 8 — iteration_mode written and readable by auto_run logic
// ===========================================================================

#[tokio::test]
async fn test_phase_8_iteration_mode_read_by_auto_run() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();
    save_state_through_phase(&prd_conn, 7);
    setup_db_through_phase(&prd_conn, 7);

    // Load state and run phase 8 directly
    let mut state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    let provider = SequentialMockProvider::new(vec![phase_8_response()]);

    let mode = prd::wizard::run_wizard_phase_8(&provider, &prd_conn, &mut state)
        .await
        .unwrap();

    assert_eq!(mode, "review_every:3");

    // Verify all 3 config values written by phase 8
    let stored_mode = dial_core::config::config_get("iteration_mode")
        .unwrap()
        .unwrap();
    let stored_cli = dial_core::config::config_get("ai_cli").unwrap().unwrap();
    let stored_timeout = dial_core::config::config_get("subagent_timeout")
        .unwrap()
        .unwrap();

    assert_eq!(stored_mode, "review_every:3");
    assert_eq!(stored_cli, "claude");
    assert_eq!(stored_timeout, "1800");

    // Verify IterationMode::from_config parses the written value correctly.
    // This is the SAME code path that auto_run() uses (orchestrator.rs:281-283).
    use dial_core::iteration::orchestrator::IterationMode;

    let parsed = IterationMode::from_config(&stored_mode);
    assert_eq!(parsed, IterationMode::ReviewEvery(3));
    assert_eq!(parsed.display_name(), "review_every:3");

    // Reproduce auto_run's exact config reading pattern end-to-end
    let auto_run_mode = dial_core::config::config_get("iteration_mode")
        .unwrap()
        .map(|s| IterationMode::from_config(&s))
        .unwrap_or(IterationMode::Autonomous);
    assert_eq!(auto_run_mode, IterationMode::ReviewEvery(3));

    // Verify other IterationMode variants parse correctly
    assert_eq!(
        IterationMode::from_config("autonomous"),
        IterationMode::Autonomous
    );
    assert_eq!(
        IterationMode::from_config("review_each"),
        IterationMode::ReviewEach
    );
    assert_eq!(
        IterationMode::from_config("review_every:5"),
        IterationMode::ReviewEvery(5)
    );
    assert_eq!(
        IterationMode::from_config("review_every:1"),
        IterationMode::ReviewEvery(1)
    );

    // Edge cases: invalid values fall back to Autonomous
    assert_eq!(
        IterationMode::from_config("review_every:0"),
        IterationMode::Autonomous
    );
    assert_eq!(
        IterationMode::from_config("review_every:abc"),
        IterationMode::Autonomous
    );
    assert_eq!(
        IterationMode::from_config("unknown_mode"),
        IterationMode::Autonomous
    );
    assert_eq!(IterationMode::from_config(""), IterationMode::Autonomous);

    // Verify no config means auto_run defaults to Autonomous
    // (simulates fresh project with no phase 8)
    dial_core::config::config_set("iteration_mode", "").unwrap();
    let default_mode = dial_core::config::config_get("iteration_mode")
        .unwrap()
        .map(|s| IterationMode::from_config(&s))
        .unwrap_or(IterationMode::Autonomous);
    assert_eq!(default_mode, IterationMode::Autonomous);

    // Verify wizard state was updated
    let updated_state = prd::wizard::load_wizard_state(&prd_conn)
        .unwrap()
        .unwrap();
    assert!(updated_state.completed_phases.contains(&8));
    assert!(updated_state.gathered_info.get("iteration_mode").is_some());
}

// ===========================================================================
// Test: Phase 4 prompt contains SPECIFICITY CHECK section
// ===========================================================================

#[test]
fn test_phase_4_prompt_contains_specificity_check() {
    let state = prd::wizard::WizardState::new("spec");
    let prompt = prd::wizard::build_phase_prompt(
        prd::wizard::WizardPhase::GapAnalysis,
        &state,
        None,
    );

    // Verify SPECIFICITY CHECK section is present
    assert!(
        prompt.contains("## SPECIFICITY CHECK"),
        "Phase 4 prompt should contain SPECIFICITY CHECK header"
    );

    // Verify vague language markers are listed
    assert!(
        prompt.contains("'should'"),
        "Prompt should flag 'should' as vague"
    );
    assert!(
        prompt.contains("'might'"),
        "Prompt should flag 'might' as vague"
    );
    assert!(
        prompt.contains("'could'"),
        "Prompt should flag 'could' as vague"
    );
    assert!(
        prompt.contains("'etc.'"),
        "Prompt should flag 'etc.' as vague"
    );
    assert!(
        prompt.contains("'various'"),
        "Prompt should flag 'various' as vague"
    );

    // Verify rating categories
    assert!(
        prompt.contains("SPECIFIC"),
        "Prompt should define SPECIFIC rating"
    );
    assert!(
        prompt.contains("NEEDS_DETAIL"),
        "Prompt should define NEEDS_DETAIL rating"
    );
    assert!(
        prompt.contains("VAGUE"),
        "Prompt should define VAGUE rating"
    );

    // Verify the critical instruction
    assert!(
        prompt.contains("Do not proceed to Phase 5 with any VAGUE sections"),
        "Prompt must contain the Phase 5 gate instruction"
    );
    assert!(
        prompt.contains("Rewrite them now with specific acceptance criteria"),
        "Prompt must instruct rewriting VAGUE sections"
    );

    // Verify JSON response format includes new fields
    assert!(
        prompt.contains("section_ratings"),
        "Prompt should request section_ratings in JSON format"
    );
    assert!(
        prompt.contains("rewritten_sections"),
        "Prompt should request rewritten_sections in JSON format"
    );

    // Verify the prompt still includes the original gap analysis fields
    assert!(prompt.contains("\"gaps\""), "Prompt should still include gaps");
    assert!(
        prompt.contains("\"contradictions\""),
        "Prompt should still include contradictions"
    );
    assert!(
        prompt.contains("\"recommendations\""),
        "Prompt should still include recommendations"
    );
}

// ===========================================================================
// Test: Phase 4 prompt includes prior context from phases 1-3
// ===========================================================================

#[test]
fn test_phase_4_prompt_with_prior_context() {
    let mut state = prd::wizard::WizardState::new("spec");
    state.gathered_info = gathered_info_through_phase(3);
    for p in 1..=3 {
        state.completed_phases.push(p);
    }

    let prompt = prd::wizard::build_phase_prompt(
        prd::wizard::WizardPhase::GapAnalysis,
        &state,
        None,
    );

    // Should include prior gathered info
    assert!(
        prompt.contains("Previously Gathered Information"),
        "Prompt should include prior context"
    );
    assert!(
        prompt.contains("WizardTestProject"),
        "Prompt should include project name from phase 1"
    );

    // Should still have specificity check
    assert!(
        prompt.contains("## SPECIFICITY CHECK"),
        "Prompt with prior context should contain SPECIFICITY CHECK"
    );
}

// ===========================================================================
// Test: parse_specificity_response extracts ratings and rewrites
// ===========================================================================

#[test]
fn test_parse_specificity_response() {
    let data = json!({
        "gaps": [{"area": "testing", "issue": "missing tests", "suggestion": "add tests"}],
        "contradictions": [],
        "recommendations": [],
        "section_ratings": [
            {"section": "vision", "rating": "SPECIFIC", "issues": []},
            {"section": "functionality", "rating": "VAGUE", "issues": ["uses 'should'", "no acceptance criteria"]},
            {"section": "technical", "rating": "NEEDS_DETAIL", "issues": ["missing performance targets"]}
        ],
        "rewritten_sections": [
            {
                "section": "functionality",
                "original": "The system should handle various user inputs",
                "rewritten": "The system accepts UTF-8 text input up to 10,000 characters. Acceptance criteria: (1) Rejects input >10,000 chars with error code 413, (2) Strips HTML tags from input, (3) Returns processed result within 200ms p95."
            }
        ]
    });

    let (ratings, rewrites) = prd::wizard::parse_specificity_response(&data);

    // Verify ratings
    assert_eq!(ratings.len(), 3);

    assert_eq!(ratings[0].section, "vision");
    assert_eq!(ratings[0].rating, "SPECIFIC");
    assert!(ratings[0].issues.is_empty());

    assert_eq!(ratings[1].section, "functionality");
    assert_eq!(ratings[1].rating, "VAGUE");
    assert_eq!(ratings[1].issues.len(), 2);
    assert_eq!(ratings[1].issues[0], "uses 'should'");
    assert_eq!(ratings[1].issues[1], "no acceptance criteria");

    assert_eq!(ratings[2].section, "technical");
    assert_eq!(ratings[2].rating, "NEEDS_DETAIL");
    assert_eq!(ratings[2].issues.len(), 1);

    // Verify rewrites
    assert_eq!(rewrites.len(), 1);
    assert_eq!(rewrites[0].section, "functionality");
    assert!(rewrites[0].original.contains("should handle various"));
    assert!(rewrites[0].rewritten.contains("UTF-8 text input"));
    assert!(rewrites[0].rewritten.contains("Acceptance criteria"));
}

// ===========================================================================
// Test: parse_specificity_response handles missing fields gracefully
// ===========================================================================

#[test]
fn test_parse_specificity_response_missing_fields() {
    // Old-style Phase 4 response without specificity fields
    let data = json!({
        "gaps": [{"area": "testing", "issue": "missing", "suggestion": "add"}],
        "contradictions": [],
        "recommendations": []
    });

    let (ratings, rewrites) = prd::wizard::parse_specificity_response(&data);

    assert!(ratings.is_empty(), "Should return empty ratings for old-style response");
    assert!(rewrites.is_empty(), "Should return empty rewrites for old-style response");
}

// ===========================================================================
// Test: parse_specificity_response handles malformed entries
// ===========================================================================

#[test]
fn test_parse_specificity_response_malformed_entries() {
    let data = json!({
        "section_ratings": [
            {"section": "valid", "rating": "SPECIFIC", "issues": []},
            {"section": "missing_rating"},
            {"rating": "VAGUE", "issues": ["no section field"]},
            {"section": "no_issues", "rating": "NEEDS_DETAIL"}
        ],
        "rewritten_sections": [
            {"section": "valid", "original": "old text", "rewritten": "new text"},
            {"section": "missing_rewritten", "original": "old"},
            {"original": "no section", "rewritten": "new"}
        ]
    });

    let (ratings, rewrites) = prd::wizard::parse_specificity_response(&data);

    // Only entries with all required fields are included
    assert_eq!(ratings.len(), 2, "Should parse valid entries and skip malformed");
    assert_eq!(ratings[0].section, "valid");
    assert_eq!(ratings[1].section, "no_issues");
    assert!(ratings[1].issues.is_empty(), "Missing issues field defaults to empty vec");

    assert_eq!(rewrites.len(), 1, "Should only parse fully valid rewrites");
    assert_eq!(rewrites[0].section, "valid");
}

// ===========================================================================
// Test: Integration — Phase 4 rewrites vague sections in prd.db
// ===========================================================================

#[tokio::test]
async fn test_phase_4_vague_section_rewrite_in_prd_db() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();

    // Pre-populate prd.db with sections (as if imported via `--from`)
    prd::prd_insert_section(&prd_conn, "1", "Overview", None, 1, 0,
        "The system should handle various tasks", 6).unwrap();
    prd::prd_insert_section(&prd_conn, "2", "Architecture", None, 1, 1,
        "Uses a modular architecture with defined interfaces", 7).unwrap();
    prd::prd_insert_section(&prd_conn, "3", "Requirements", None, 1, 2,
        "Some requirements might be added later etc.", 7).unwrap();

    // Set up wizard state through phase 3
    save_state_through_phase(&prd_conn, 3);

    // Phase 4 response with rewrites for vague sections
    let phase_4_with_rewrites = serde_json::to_string(&json!({
        "gaps": [
            {"area": "requirements", "issue": "Acceptance criteria missing", "suggestion": "Add measurable criteria"}
        ],
        "contradictions": [],
        "recommendations": [
            {"topic": "specificity", "recommendation": "All sections need concrete acceptance criteria"}
        ],
        "section_ratings": [
            {"section": "Overview", "rating": "VAGUE", "issues": ["uses 'should'", "uses 'various'"]},
            {"section": "Architecture", "rating": "SPECIFIC", "issues": []},
            {"section": "Requirements", "rating": "VAGUE", "issues": ["uses 'some'", "uses 'might'", "uses 'etc.'"]}
        ],
        "rewritten_sections": [
            {
                "section": "Overview",
                "original": "The system should handle various tasks",
                "rewritten": "The system processes exactly 3 task types: build validation, test execution, and deployment. Acceptance criteria: (1) Each task completes within configured timeout, (2) Failed tasks produce structured error output, (3) Task queue supports FIFO ordering with priority override."
            },
            {
                "section": "Requirements",
                "original": "Some requirements might be added later etc.",
                "rewritten": "Functional requirements: (1) User authentication via API key with 256-bit entropy, (2) Task scheduling supports cron expressions, (3) Results persisted to SQLite with WAL mode. Non-functional: (1) p95 latency < 500ms, (2) Supports 100 concurrent tasks."
            }
        ]
    })).unwrap();

    // Run with phase 4 rewrite response + phase 5 generate response
    seed_spec_sections();
    let responses = vec![
        phase_4_with_rewrites,
        phase_5_response(),
    ];
    let provider = SequentialMockProvider::new(responses);

    let _result = prd::wizard::run_wizard(&provider, &prd_conn, "spec", None, true, false)
        .await
        .unwrap();

    // Verify that the vague sections were rewritten in prd.db
    // Note: Phase 5 deletes all sections and recreates them, so we need to check
    // the gathered_info to verify the rewrites were parsed correctly.
    let state = prd::wizard::load_wizard_state(&prd_conn).unwrap().unwrap();
    let gap_data = &state.gathered_info["gap_analysis"];

    // Verify section_ratings are stored
    let ratings = gap_data.get("section_ratings").unwrap().as_array().unwrap();
    assert_eq!(ratings.len(), 3);
    assert_eq!(ratings[0]["rating"].as_str().unwrap(), "VAGUE");
    assert_eq!(ratings[1]["rating"].as_str().unwrap(), "SPECIFIC");
    assert_eq!(ratings[2]["rating"].as_str().unwrap(), "VAGUE");

    // Verify rewritten_sections are stored
    let rewrites = gap_data.get("rewritten_sections").unwrap().as_array().unwrap();
    assert_eq!(rewrites.len(), 2);
    assert_eq!(rewrites[0]["section"].as_str().unwrap(), "Overview");
    assert!(rewrites[0]["rewritten"].as_str().unwrap().contains("Acceptance criteria"));
    assert_eq!(rewrites[1]["section"].as_str().unwrap(), "Requirements");
    assert!(rewrites[1]["rewritten"].as_str().unwrap().contains("Functional requirements"));
}

// ===========================================================================
// Test: apply_specificity_rewrites updates matching sections in prd.db
// ===========================================================================

#[tokio::test]
async fn test_apply_specificity_rewrites_updates_sections() {
    let _lock = lock();
    let (_engine, _tmp, _guard) = setup_engine().await;

    let prd_conn = prd::get_or_init_prd_db().unwrap();

    // Insert sections
    prd::prd_insert_section(&prd_conn, "1", "Overview", None, 1, 0,
        "The system should handle various tasks", 6).unwrap();
    prd::prd_insert_section(&prd_conn, "2", "Architecture", None, 1, 1,
        "Clean architecture with layers", 5).unwrap();

    let rewrites = vec![
        prd::wizard::RewrittenSection {
            section: "Overview".to_string(),
            original: "The system should handle various tasks".to_string(),
            rewritten: "The system processes build, test, and deploy tasks with <500ms p95 latency.".to_string(),
        },
        prd::wizard::RewrittenSection {
            section: "Nonexistent Section".to_string(),
            original: "original text".to_string(),
            rewritten: "new text".to_string(),
        },
    ];

    let updated = prd::wizard::apply_specificity_rewrites(&prd_conn, &rewrites).unwrap();
    assert_eq!(updated, 1, "Should update 1 section (Overview), skip nonexistent");

    // Verify the content was updated
    let section = prd::prd_get_section(&prd_conn, "1").unwrap().unwrap();
    assert_eq!(section.title, "Overview");
    assert!(
        section.content.contains("build, test, and deploy"),
        "Content should be updated to rewritten text"
    );
    assert!(
        section.content.contains("<500ms p95"),
        "Content should have concrete acceptance criteria"
    );

    // Verify word count was recalculated
    let expected_wc = "The system processes build, test, and deploy tasks with <500ms p95 latency."
        .split_whitespace().count() as i32;
    assert_eq!(section.word_count, expected_wc);

    // Verify the other section was NOT modified
    let arch_section = prd::prd_get_section(&prd_conn, "2").unwrap().unwrap();
    assert_eq!(arch_section.content, "Clean architecture with layers");
}
