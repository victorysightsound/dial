use crate::errors::Result;
use crate::prd::templates::{get_template, Template};
use rusqlite::{params, Connection};
use serde_json::Value as JsonValue;

/// Wizard phases for PRD creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardPhase {
    Vision = 1,
    Functionality = 2,
    Technical = 3,
    GapAnalysis = 4,
    Generate = 5,
}

impl WizardPhase {
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            1 => Some(Self::Vision),
            2 => Some(Self::Functionality),
            3 => Some(Self::Technical),
            4 => Some(Self::GapAnalysis),
            5 => Some(Self::Generate),
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
    }
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
