use crate::errors::{DialError, Result};
use crate::prd::templates::{get_template, Template};
use crate::provider::{Provider, ProviderRequest};
use rusqlite::{params, Connection};
use serde_json::Value as JsonValue;
use std::path::Path;

/// Wizard phases for PRD creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardPhase {
    Vision = 1,
    Functionality = 2,
    Technical = 3,
    GapAnalysis = 4,
    Generate = 5,
    TaskReview = 6,
    BuildTestConfig = 7,
    IterationMode = 8,
    Launch = 9,
}

impl WizardPhase {
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            1 => Some(Self::Vision),
            2 => Some(Self::Functionality),
            3 => Some(Self::Technical),
            4 => Some(Self::GapAnalysis),
            5 => Some(Self::Generate),
            6 => Some(Self::TaskReview),
            7 => Some(Self::BuildTestConfig),
            8 => Some(Self::IterationMode),
            9 => Some(Self::Launch),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Vision => "Vision",
            Self::Functionality => "Functionality",
            Self::Technical => "Technical",
            Self::GapAnalysis => "Gap Analysis",
            Self::Generate => "Generate",
            Self::TaskReview => "Task Review",
            Self::BuildTestConfig => "Build & Test Config",
            Self::IterationMode => "Iteration Mode",
            Self::Launch => "Launch",
        }
    }

    pub fn next(&self) -> Option<Self> {
        Self::from_i32(*self as i32 + 1)
    }
}

/// Persistent wizard state for pause/resume.
#[derive(Debug, Clone)]
pub struct WizardState {
    pub id: i64,
    pub current_phase: WizardPhase,
    pub completed_phases: Vec<i32>,
    pub gathered_info: JsonValue,
    pub template: String,
    pub started_at: String,
    pub updated_at: Option<String>,
}

impl WizardState {
    pub fn new(template: &str) -> Self {
        Self {
            id: 0,
            current_phase: WizardPhase::Vision,
            completed_phases: Vec::new(),
            gathered_info: serde_json::json!({}),
            template: template.to_string(),
            started_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            updated_at: None,
        }
    }

    pub fn mark_phase_complete(&mut self, phase: WizardPhase) {
        let phase_num = phase as i32;
        if !self.completed_phases.contains(&phase_num) {
            self.completed_phases.push(phase_num);
        }
        if let Some(next) = phase.next() {
            self.current_phase = next;
        }
    }

    pub fn set_phase_data(&mut self, phase: WizardPhase, data: JsonValue) {
        self.gathered_info[phase.name().to_lowercase().replace(' ', "_")] = data;
    }
}

/// Save wizard state to the database (upsert).
pub fn save_wizard_state(conn: &Connection, state: &WizardState) -> Result<()> {
    let completed_json = serde_json::to_string(&state.completed_phases)
        .unwrap_or_else(|_| "[]".to_string());
    let info_json = serde_json::to_string(&state.gathered_info)
        .unwrap_or_else(|_| "{}".to_string());

    if state.id > 0 {
        conn.execute(
            "UPDATE wizard_state SET current_phase = ?1, completed_phases = ?2,
             gathered_info = ?3, template = ?4, updated_at = strftime('%Y-%m-%dT%H:%M:%S', 'now')
             WHERE id = ?5",
            params![
                state.current_phase as i32,
                completed_json,
                info_json,
                state.template,
                state.id,
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO wizard_state (current_phase, completed_phases, gathered_info, template)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                state.current_phase as i32,
                completed_json,
                info_json,
                state.template,
            ],
        )?;
    }
    Ok(())
}

/// Load the most recent wizard state.
pub fn load_wizard_state(conn: &Connection) -> Result<Option<WizardState>> {
    let mut stmt = conn.prepare(
        "SELECT id, current_phase, completed_phases, gathered_info, template, started_at, updated_at
         FROM wizard_state ORDER BY id DESC LIMIT 1",
    )?;

    let result = stmt
        .query_row([], |row| {
            let phase_num: i32 = row.get(1)?;
            let completed_str: String = row.get(2)?;
            let info_str: String = row.get(3)?;

            Ok(WizardState {
                id: row.get(0)?,
                current_phase: WizardPhase::from_i32(phase_num).unwrap_or(WizardPhase::Vision),
                completed_phases: serde_json::from_str(&completed_str).unwrap_or_default(),
                gathered_info: serde_json::from_str(&info_str)
                    .unwrap_or_else(|_| serde_json::json!({})),
                template: row.get(4)?,
                started_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .ok();

    Ok(result)
}

/// Clear all wizard state (for restart).
pub fn clear_wizard_state(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM wizard_state", [])?;
    Ok(())
}

/// Build a prompt for a wizard phase.
///
/// The prompt includes the system instruction, accumulated context from
/// prior phases, and the current phase's questions. The response format
/// is always JSON.
pub fn build_phase_prompt(
    phase: WizardPhase,
    state: &WizardState,
    existing_doc: Option<&str>,
) -> String {
    let template = get_template(&state.template);
    let template_context = template
        .map(|t| format_template_context(t))
        .unwrap_or_default();

    let prior_context = if state.gathered_info.is_object()
        && !state.gathered_info.as_object().unwrap().is_empty()
    {
        format!(
            "\n## Previously Gathered Information\n```json\n{}\n```\n",
            serde_json::to_string_pretty(&state.gathered_info).unwrap_or_default()
        )
    } else {
        String::new()
    };

    let doc_context = existing_doc
        .map(|doc| format!("\n## Existing Document\n{}\n", doc))
        .unwrap_or_default();

    match phase {
        WizardPhase::Vision => format!(
            r#"You are helping create a Product Requirements Document (PRD).

Phase 1: Vision & Problem

{template_context}{prior_context}{doc_context}

Answer these questions in JSON format:
{{
  "project_name": "short name for the project",
  "elevator_pitch": "one sentence describing what this is",
  "problem_statement": "what problem does this solve and why does it matter",
  "target_users": ["who will use this"],
  "success_criteria": ["how do we know it's working"],
  "scope_exclusions": ["what this should NOT do"]
}}

Respond ONLY with valid JSON."#
        ),
        WizardPhase::Functionality => format!(
            r#"You are helping create a PRD. Phase 2: Functionality.

{template_context}{prior_context}{doc_context}

Based on the vision, define the features in JSON format:
{{
  "mvp_features": [
    {{"name": "feature name", "description": "what it does", "priority": 1}}
  ],
  "deferred_features": [
    {{"name": "feature name", "description": "what it does", "rationale": "why deferred"}}
  ],
  "user_workflows": [
    {{"name": "workflow name", "steps": ["step 1", "step 2"]}}
  ]
}}

Respond ONLY with valid JSON."#
        ),
        WizardPhase::Technical => format!(
            r#"You are helping create a PRD. Phase 3: Technical Details.

{template_context}{prior_context}{doc_context}

Define the technical architecture in JSON format:
{{
  "data_model": [
    {{"entity": "name", "fields": ["field1: type", "field2: type"], "relationships": ["relates to X"]}}
  ],
  "integrations": [
    {{"service": "name", "purpose": "why needed", "api_type": "REST/GraphQL/etc"}}
  ],
  "platform": {{"languages": ["Rust"], "frameworks": [], "database": "SQLite", "hosting": ""}},
  "constraints": ["constraint 1", "constraint 2"],
  "performance_requirements": ["requirement 1"]
}}

Respond ONLY with valid JSON."#
        ),
        WizardPhase::GapAnalysis => format!(
            r#"You are a senior software architect reviewing a PRD for completeness.

{template_context}{prior_context}{doc_context}

Review everything gathered so far and identify:
1. Missing details that would block implementation
2. Contradictions between sections
3. Ambiguous requirements that need clarification
4. Edge cases not covered
5. Security or performance concerns not addressed

Respond in JSON format:
{{
  "gaps": [
    {{"area": "which section/topic", "issue": "what's missing or unclear", "suggestion": "how to resolve it"}}
  ],
  "contradictions": [
    {{"between": ["section A", "section B"], "issue": "what conflicts"}}
  ],
  "recommendations": [
    {{"topic": "what to consider", "recommendation": "suggested approach"}}
  ]
}}

Respond ONLY with valid JSON."#
        ),
        WizardPhase::Generate => format!(
            r#"You are generating a structured PRD from gathered information.

{template_context}{prior_context}{doc_context}

Generate the complete PRD content as a JSON object where each key is a section title
from the template and each value is the markdown content for that section.
Include all relevant information gathered from prior phases.

Respond in JSON format:
{{
  "sections": [
    {{"title": "section title", "content": "full markdown content for this section"}}
  ],
  "terminology": [
    {{"term": "canonical term", "definition": "what it means", "category": "domain/technical/workflow"}}
  ]
}}

Respond ONLY with valid JSON."#
        ),
        WizardPhase::TaskReview => {
            // Phase 6 uses build_task_review_prompt() for the full prompt
            // when tasks are available. This fallback uses only gathered_info.
            build_task_review_prompt(&[], &state.gathered_info)
        }
        WizardPhase::BuildTestConfig => format!(
            r#"You are configuring build and test commands for a project.

{prior_context}

Based on the technical details (languages, frameworks, platform), suggest build and test commands
and validation pipeline steps.

Respond in JSON format:
{{
  "build_cmd": "cargo build",
  "test_cmd": "cargo test",
  "pipeline_steps": [
    {{"name": "lint", "command": "cargo clippy", "order": 1, "required": true, "timeout": 120}},
    {{"name": "build", "command": "cargo build", "order": 2, "required": true, "timeout": 300}},
    {{"name": "test", "command": "cargo test", "order": 3, "required": true, "timeout": 300}}
  ],
  "build_timeout": 600,
  "test_timeout": 600,
  "rationale": "why these commands"
}}

Respond ONLY with valid JSON."#
        ),
        WizardPhase::IterationMode => format!(
            r#"You are recommending an iteration mode for autonomous development.

{prior_context}

Available modes:
- autonomous: Run all tasks, commit on pass, no stops
- review_every:N: Pause for review after every N completed tasks
- review_each: Pause after every task for approval

Based on the project scope, task count, and complexity, recommend a mode.

Respond in JSON format:
{{
  "recommended_mode": "autonomous",
  "review_interval": null,
  "ai_cli": "claude",
  "subagent_timeout": 1800,
  "rationale": "why this mode"
}}

Respond ONLY with valid JSON."#
        ),
        WizardPhase::Launch => {
            // Phase 9 is not an AI provider call — it prints a summary.
            // This prompt is not expected to be sent to a provider.
            format!(
                "Launch phase: no AI prompt needed. Project is ready for `dial auto-run`.\n{prior_context}"
            )
        }
    }
}

/// Run the wizard through phases 1-3 (information gathering).
///
/// If `from_doc` is provided, the existing document content is included
/// alongside each phase prompt so the AI can extract information from it.
/// State is persisted after each phase for pause/resume.
pub async fn run_wizard_phases_1_3(
    provider: &dyn Provider,
    prd_conn: &Connection,
    state: &mut WizardState,
    from_doc: Option<&str>,
) -> Result<()> {
    let phases = [WizardPhase::Vision, WizardPhase::Functionality, WizardPhase::Technical];

    for phase in &phases {
        // Skip already completed phases (for resume)
        if state.completed_phases.contains(&(*phase as i32)) {
            continue;
        }

        state.current_phase = *phase;
        save_wizard_state(prd_conn, state)?;

        let prompt = build_phase_prompt(*phase, state, from_doc);
        let response = execute_wizard_prompt(provider, &prompt).await?;

        // Parse JSON response
        let data = parse_json_response(&response, provider, &prompt).await?;
        state.set_phase_data(*phase, data);
        state.mark_phase_complete(*phase);
        save_wizard_state(prd_conn, state)?;
    }

    Ok(())
}

/// Load existing document content for --from mode.
pub fn load_existing_doc(from_path: &str) -> Result<String> {
    let path = Path::new(from_path);
    if !path.exists() {
        return Err(DialError::UserError(format!("File not found: {}", from_path)));
    }
    let content = std::fs::read_to_string(path)?;
    Ok(content)
}

/// Execute a wizard prompt against the provider.
async fn execute_wizard_prompt(provider: &dyn Provider, prompt: &str) -> Result<String> {
    let request = ProviderRequest {
        prompt: prompt.to_string(),
        work_dir: std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        max_tokens: Some(4096),
        model: None,
        timeout_secs: Some(120),
    };

    let response = provider.execute(request).await?;

    if !response.success {
        return Err(DialError::WizardError(format!(
            "Provider returned failure: {}",
            response.output
        )));
    }

    Ok(response.output)
}

/// Parse a JSON response from the provider, with one retry on failure.
async fn parse_json_response(
    response: &str,
    provider: &dyn Provider,
    original_prompt: &str,
) -> Result<JsonValue> {
    // Try to extract JSON from the response (it might have markdown wrapping)
    let json_str = extract_json(response);

    match serde_json::from_str::<JsonValue>(&json_str) {
        Ok(value) => Ok(value),
        Err(_) => {
            // Retry with a clarification prompt
            let retry_prompt = format!(
                "{}\n\nYour previous response was not valid JSON. Please respond with ONLY a valid JSON object. No markdown, no explanation, just JSON.",
                original_prompt
            );
            let retry_response = execute_wizard_prompt(provider, &retry_prompt).await?;
            let retry_json = extract_json(&retry_response);
            serde_json::from_str::<JsonValue>(&retry_json)
                .map_err(|e| DialError::WizardError(format!("Failed to parse JSON response: {}", e)))
        }
    }
}

/// Run wizard phases 4-5 (gap analysis and generation).
///
/// Phase 4: Sends all gathered info to the provider for gap analysis.
/// Phase 5: Generates PRD sections, inserts into prd.db, extracts terminology,
///          and creates linked DIAL tasks.
pub async fn run_wizard_phases_4_5(
    provider: &dyn Provider,
    prd_conn: &Connection,
    state: &mut WizardState,
    from_doc: Option<&str>,
) -> Result<(usize, usize)> {
    let mut sections_generated = 0;
    let mut tasks_generated = 0;

    // Phase 4: Gap Analysis
    if !state.completed_phases.contains(&(WizardPhase::GapAnalysis as i32)) {
        state.current_phase = WizardPhase::GapAnalysis;
        save_wizard_state(prd_conn, state)?;

        let prompt = build_phase_prompt(WizardPhase::GapAnalysis, state, from_doc);
        let response = execute_wizard_prompt(provider, &prompt).await?;
        let data = parse_json_response(&response, provider, &prompt).await?;
        state.set_phase_data(WizardPhase::GapAnalysis, data);
        state.mark_phase_complete(WizardPhase::GapAnalysis);
        save_wizard_state(prd_conn, state)?;
    }

    // Phase 5: Generate
    if !state.completed_phases.contains(&(WizardPhase::Generate as i32)) {
        state.current_phase = WizardPhase::Generate;
        save_wizard_state(prd_conn, state)?;

        let prompt = build_phase_prompt(WizardPhase::Generate, state, from_doc);
        let response = execute_wizard_prompt(provider, &prompt).await?;
        let data = parse_json_response(&response, provider, &prompt).await?;

        // Insert generated sections into prd.db
        if let Some(sections) = data.get("sections").and_then(|s| s.as_array()) {
            crate::prd::prd_delete_all_sections(prd_conn)?;

            for (i, section) in sections.iter().enumerate() {
                let title = section.get("title").and_then(|t| t.as_str()).unwrap_or("Untitled");
                let content = section.get("content").and_then(|c| c.as_str()).unwrap_or("");
                let word_count = content.split_whitespace().count() as i32;

                // Generate section_id from position
                let section_id = format!("{}", i + 1);

                crate::prd::prd_insert_section(
                    prd_conn,
                    &section_id,
                    title,
                    None,
                    1,
                    i as i32,
                    content,
                    word_count,
                )?;
                sections_generated += 1;
            }
        }

        // Extract and store terminology
        if let Some(terms) = data.get("terminology").and_then(|t| t.as_array()) {
            for term in terms {
                let canonical = term.get("term").and_then(|t| t.as_str()).unwrap_or_default();
                let definition = term.get("definition").and_then(|d| d.as_str()).unwrap_or_default();
                let category = term.get("category").and_then(|c| c.as_str()).unwrap_or("general");

                if !canonical.is_empty() {
                    let _ = crate::prd::prd_add_term(
                        prd_conn,
                        canonical,
                        "[]",
                        definition,
                        category,
                        None,
                    );
                }
            }
        }

        // Generate DIAL tasks from sections
        let phase_conn = crate::db::get_db(None)?;
        if let Some(sections) = data.get("sections").and_then(|s| s.as_array()) {
            for (i, section) in sections.iter().enumerate() {
                let title = section.get("title").and_then(|t| t.as_str()).unwrap_or("Untitled");
                let desc = format!("Implement: {}", title);
                let priority = (i + 1) as i32;

                phase_conn.execute(
                    "INSERT INTO tasks (description, status, priority, spec_section_id)
                     VALUES (?1, 'pending', ?2, ?3)",
                    rusqlite::params![desc, priority, (i + 1) as i64],
                )?;
                tasks_generated += 1;
            }
        }

        state.set_phase_data(WizardPhase::Generate, data);
        state.mark_phase_complete(WizardPhase::Generate);
        save_wizard_state(prd_conn, state)?;
    }

    Ok((sections_generated, tasks_generated))
}

/// Run the complete wizard (all 5 phases).
pub async fn run_wizard(
    provider: &dyn Provider,
    prd_conn: &Connection,
    template: &str,
    from_doc: Option<&str>,
    resume: bool,
) -> Result<(usize, usize)> {
    let mut state = if resume {
        load_wizard_state(prd_conn)?
            .unwrap_or_else(|| WizardState::new(template))
    } else {
        clear_wizard_state(prd_conn)?;
        WizardState::new(template)
    };

    // Validate template exists
    if get_template(&state.template).is_none() {
        return Err(DialError::TemplateNotFound(state.template.clone()));
    }

    // Phases 1-3: Information gathering
    run_wizard_phases_1_3(provider, prd_conn, &mut state, from_doc).await?;

    // Phases 4-5: Gap analysis and generation
    let result = run_wizard_phases_4_5(provider, prd_conn, &mut state, from_doc).await?;

    Ok(result)
}

/// Build the phase 6 (Task Review) prompt with actual task list and PRD context.
///
/// Unlike other phases that only use `gathered_info` from prior phases, phase 6
/// requires the actual task list from the database (generated by phase 5)
/// formatted alongside the full PRD context.
///
/// # Arguments
/// * `tasks` - Tuples of (id, description, priority, prd_section_id) from the DB
/// * `gathered_info` - Accumulated wizard state from phases 1-5
pub fn build_task_review_prompt(
    tasks: &[(i64, String, i32, Option<String>)],
    gathered_info: &JsonValue,
) -> String {
    let task_list = if tasks.is_empty() {
        "No tasks have been generated yet.".to_string()
    } else {
        let items: Vec<String> = tasks
            .iter()
            .map(|(id, desc, priority, section)| {
                let section_str = section.as_deref().unwrap_or("none");
                format!(
                    "  - [#{}] P{} (section: {}) {}",
                    id, priority, section_str, desc
                )
            })
            .collect();
        items.join("\n")
    };

    let prd_context = if gathered_info.is_object()
        && !gathered_info.as_object().unwrap().is_empty()
    {
        format!(
            "\n## Full PRD Context (from phases 1-5)\n```json\n{}\n```\n",
            serde_json::to_string_pretty(gathered_info).unwrap_or_default()
        )
    } else {
        String::new()
    };

    format!(
        r#"You are a senior software architect reviewing and refining a task list generated from a PRD.

## Current Task List (generated from PRD)
{task_list}
{prd_context}
Review the tasks above and refine them:
1. Reorder by logical implementation sequence (foundation first, then features, then polish)
2. Add any missing tasks needed for a complete implementation
3. Remove redundant or overly-granular tasks
4. Set dependency relationships using 0-based indices into your output tasks array
5. Assign realistic priorities (1 = implement first, higher numbers = implement later)

Each task should be roughly one commit's worth of work (~30 minutes).
In the `depends_on` array, use 0-based indices referring to other tasks in YOUR output array.
For example, if the task at index 2 depends on the task at index 0, set `"depends_on": [0]`.

Respond in JSON format:
{{
  "tasks": [
    {{"description": "task description", "priority": 1, "spec_section": "1.2", "depends_on": [], "rationale": "why this order"}}
  ],
  "removed": [
    {{"original": "task that was removed", "reason": "why"}}
  ],
  "added": [
    {{"description": "new task", "reason": "why it was missing"}}
  ]
}}

Respond ONLY with valid JSON."#
    )
}

/// Read all pending/in-progress tasks from the DIAL database as tuples.
fn read_task_list(conn: &Connection) -> Result<Vec<(i64, String, i32, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT id, description, priority, prd_section_id
         FROM tasks WHERE status IN ('pending', 'in_progress')
         ORDER BY priority, id",
    )?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(rows)
}

/// Run wizard phase 6: Task Review.
///
/// Reads tasks generated by phase 5 from the DIAL database, sends them
/// to the AI provider for review and refinement, then replaces the original
/// tasks with the reviewed versions including dependency relationships.
///
/// Returns (tasks_kept, tasks_added, tasks_removed).
pub async fn run_wizard_phase_6(
    provider: &dyn Provider,
    prd_conn: &Connection,
    state: &mut WizardState,
) -> Result<(usize, usize, usize)> {
    if state.completed_phases.contains(&(WizardPhase::TaskReview as i32)) {
        return Ok((0, 0, 0));
    }

    state.current_phase = WizardPhase::TaskReview;
    save_wizard_state(prd_conn, state)?;

    // 1. Read existing tasks from the DIAL database
    let phase_conn = crate::db::get_db(None)?;
    let tasks = read_task_list(&phase_conn)?;

    // 2. Build the prompt with task list and PRD context
    let prompt = build_task_review_prompt(&tasks, &state.gathered_info);

    // 3. Send to provider and parse JSON response
    let response = execute_wizard_prompt(provider, &prompt).await?;
    let data = parse_json_response(&response, provider, &prompt).await?;

    // 4. Replace tasks in the database with reviewed versions
    let (kept, added, removed) = apply_task_review(&phase_conn, &data)?;

    // 5. Store the review data in wizard state
    state.set_phase_data(WizardPhase::TaskReview, data);
    state.mark_phase_complete(WizardPhase::TaskReview);
    save_wizard_state(prd_conn, state)?;

    Ok((kept, added, removed))
}

/// Apply the AI-reviewed task list to the database.
///
/// Deletes all existing pending tasks (from phase 5), inserts the reviewed
/// task list, and sets up dependency relationships between them.
///
/// Returns (tasks_kept, tasks_added, tasks_removed) counts.
fn apply_task_review(
    conn: &Connection,
    review_data: &JsonValue,
) -> Result<(usize, usize, usize)> {
    // Count removals from the response
    let removed_count = review_data
        .get("removed")
        .and_then(|r| r.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // Count additions from the response
    let added_count = review_data
        .get("added")
        .and_then(|a| a.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    let tasks = match review_data.get("tasks").and_then(|t| t.as_array()) {
        Some(t) => t,
        None => {
            return Err(DialError::WizardError(
                "Task review response missing 'tasks' array".to_string(),
            ))
        }
    };

    // Delete existing pending/in-progress tasks and their dependencies
    conn.execute(
        "DELETE FROM task_dependencies WHERE task_id IN (
            SELECT id FROM tasks WHERE status IN ('pending', 'in_progress')
        )",
        [],
    )?;
    conn.execute(
        "DELETE FROM tasks WHERE status IN ('pending', 'in_progress')",
        [],
    )?;

    // Insert reviewed tasks and collect their new IDs (indexed by position)
    let mut new_task_ids: Vec<i64> = Vec::with_capacity(tasks.len());

    for task in tasks {
        let description = task
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("Untitled task");
        let priority = task
            .get("priority")
            .and_then(|p| p.as_i64())
            .unwrap_or(5) as i32;
        let spec_section = task
            .get("spec_section")
            .and_then(|s| s.as_str());
        let prd_section_id = spec_section.map(|s| s.to_string());

        conn.execute(
            "INSERT INTO tasks (description, status, priority, prd_section_id)
             VALUES (?1, 'pending', ?2, ?3)",
            params![description, priority, prd_section_id],
        )?;
        new_task_ids.push(conn.last_insert_rowid());
    }

    // Set up dependency relationships using 0-based indices
    for (i, task) in tasks.iter().enumerate() {
        if let Some(deps) = task.get("depends_on").and_then(|d| d.as_array()) {
            let task_id = new_task_ids[i];
            for dep in deps {
                if let Some(dep_idx) = dep.as_u64() {
                    let dep_idx = dep_idx as usize;
                    if dep_idx < new_task_ids.len() && dep_idx != i {
                        let depends_on_id = new_task_ids[dep_idx];
                        conn.execute(
                            "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id)
                             VALUES (?1, ?2)",
                            params![task_id, depends_on_id],
                        )?;
                    }
                }
            }
        }
    }

    let kept_count = tasks.len().saturating_sub(added_count);

    Ok((kept_count, added_count, removed_count))
}

/// Extract JSON from a response that might be wrapped in markdown code blocks.
fn extract_json(text: &str) -> String {
    let trimmed = text.trim();

    // Try to find JSON in code block
    if let Some(start) = trimmed.find("```json") {
        let after_marker = &trimmed[start + 7..];
        if let Some(end) = after_marker.find("```") {
            return after_marker[..end].trim().to_string();
        }
    }

    // Try plain code block
    if let Some(start) = trimmed.find("```") {
        let after_marker = &trimmed[start + 3..];
        if let Some(end) = after_marker.find("```") {
            let inner = after_marker[..end].trim();
            if inner.starts_with('{') || inner.starts_with('[') {
                return inner.to_string();
            }
        }
    }

    // Return as-is if it looks like JSON
    trimmed.to_string()
}

fn format_template_context(template: &Template) -> String {
    let sections: Vec<String> = template
        .sections
        .iter()
        .map(|s| {
            let indent = "  ".repeat((s.level - 1) as usize);
            format!("{}{} ({})", indent, s.title, s.prompt_hint)
        })
        .collect();

    format!(
        "## Template: {} ({})\nExpected sections:\n{}\n",
        template.name,
        template.description,
        sections.join("\n")
    )
}
