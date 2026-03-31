use crate::budget;
use crate::command_safety::sanitize_shell_command;
use crate::errors::{DialError, Result};
use crate::event::Event;
use crate::output::print_warning;
use crate::prd::templates::{get_template, Template};
use crate::provider::{Provider, ProviderRequest};
use rusqlite::{params, Connection};
use serde_json::Value as JsonValue;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// Result from running the wizard.
///
/// Captures outputs from all phases so callers can emit events or inspect results.
#[derive(Debug, Clone, Default)]
pub struct WizardResult {
    pub sections_generated: usize,
    pub tasks_generated: usize,
    pub tasks_kept: usize,
    pub tasks_added: usize,
    pub tasks_removed: usize,
    pub sizing_summary: SizingSummary,
    pub build_cmd: String,
    pub test_cmd: String,
    pub pipeline_steps: usize,
    pub test_tasks_added: usize,
    pub iteration_mode: String,
    pub project_name: String,
    pub task_count: usize,
    pub ai_cli: String,
}

#[derive(Debug, Clone, Default)]
pub struct LaunchSummary {
    pub project_name: String,
    pub task_count: usize,
    pub build_cmd: String,
    pub test_cmd: String,
    pub iteration_mode: String,
    pub ai_cli: String,
}

pub type WizardEventSink = Arc<dyn Fn(Event) + Send + Sync>;

fn emit_wizard_event(event_sink: Option<&WizardEventSink>, event: Event) {
    if let Some(sink) = event_sink {
        sink(event);
    }
}

fn json_context_block(title: &str, value: &JsonValue) -> String {
    format!(
        "\n## {}\n```json\n{}\n```\n",
        title,
        serde_json::to_string_pretty(value).unwrap_or_default()
    )
}

fn selected_gathered_context(gathered_info: &JsonValue, blocks: &[(&str, &str)]) -> String {
    let mut result = String::new();
    for (key, label) in blocks {
        if let Some(value) = gathered_info.get(*key) {
            result.push_str(&json_context_block(label, value));
        }
    }
    result
}

fn truncate_for_prompt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }

    let truncated: String = trimmed.chars().take(max_chars).collect();
    format!("{}...", truncated.trim_end())
}

fn maybe_store_source_requirements(state: &mut WizardState, from_doc: Option<&str>) -> bool {
    let Some(doc) = from_doc else {
        return false;
    };

    if state.gathered_info.get("source_requirements").is_some() {
        return false;
    }

    state.gathered_info["source_requirements"] = extract_source_requirements(doc);
    true
}

fn extract_source_requirements(existing_doc: &str) -> JsonValue {
    let sections = parse_markdown_sections(existing_doc);
    let mut source = serde_json::Map::new();

    for (field, headings) in [
        ("goal", &["goal", "summary", "objective"][..]),
        (
            "repository_context",
            &["repository context", "project context", "context"][..],
        ),
        (
            "requested_outcome",
            &["requested outcome", "requested outcomes", "mvp"][..],
        ),
        ("hard_constraints", &["hard constraints", "constraints"][..]),
        (
            "acceptance_checks",
            &["acceptance checks", "acceptance criteria"][..],
        ),
    ] {
        if let Some(summary) = find_markdown_section(&sections, headings) {
            source.insert(field.to_string(), JsonValue::String(summary));
        }
    }

    if source.is_empty() {
        source.insert(
            "source_excerpt".to_string(),
            JsonValue::String(summarize_markdown_excerpt(existing_doc, 18, 1800)),
        );
    }

    JsonValue::Object(source)
}

fn parse_markdown_sections(markdown: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current_heading = String::new();
    let mut current_lines = Vec::new();

    for line in markdown.lines() {
        let trimmed = line.trim_end();
        let heading = trimmed.trim();
        if heading.starts_with('#') {
            if !current_heading.is_empty() || !current_lines.is_empty() {
                sections.push((current_heading.clone(), current_lines.join("\n")));
                current_lines.clear();
            }
            current_heading = heading.trim_start_matches('#').trim().to_string();
        } else {
            current_lines.push(trimmed.to_string());
        }
    }

    if !current_heading.is_empty() || !current_lines.is_empty() {
        sections.push((current_heading, current_lines.join("\n")));
    }

    sections
}

fn find_markdown_section(
    sections: &[(String, String)],
    heading_candidates: &[&str],
) -> Option<String> {
    sections.iter().find_map(|(heading, content)| {
        let normalized_heading = normalize_quality_text(heading);
        if heading_candidates.iter().any(|candidate| {
            normalized_heading == *candidate || normalized_heading.contains(candidate)
        }) {
            let summary = summarize_markdown_excerpt(content, 14, 1400);
            if !summary.is_empty() {
                Some(summary)
            } else {
                None
            }
        } else {
            None
        }
    })
}

fn summarize_markdown_excerpt(markdown: &str, max_lines: usize, max_chars: usize) -> String {
    let mut lines = Vec::new();
    let mut total_chars = 0usize;
    let mut in_code_block = false;

    for raw_line in markdown.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block || trimmed.is_empty() {
            continue;
        }

        let normalized = if trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed
                .chars()
                .next()
                .map(|ch| ch.is_ascii_digit())
                .unwrap_or(false)
        {
            trimmed.to_string()
        } else {
            trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
        };

        if normalized.is_empty() {
            continue;
        }

        let projected = total_chars + normalized.len() + 1;
        if projected > max_chars {
            let remaining = max_chars.saturating_sub(total_chars);
            if remaining > 4 {
                lines.push(format!(
                    "{}...",
                    truncate_for_prompt(&normalized, remaining - 3)
                ));
            }
            break;
        }

        total_chars = projected;
        lines.push(normalized);
        if lines.len() >= max_lines {
            break;
        }
    }

    lines.join("\n")
}

const EXACT_PLACEHOLDER_PHRASES: &[&str] = &[
    "feature name",
    "workflow name",
    "service name",
    "entity name",
    "field1",
    "field 1",
    "field2",
    "field 2",
    "placeholder",
    "todo",
    "tbd",
];

const SUBSTRING_PLACEHOLDER_PHRASES: &[&str] = &[
    "as defined in task",
    "as finalized in task",
    "as described in task",
    "first entity",
    "second entity",
    "third entity",
    "constraint 1",
    "constraint 2",
    "requirement 1",
    "requirement 2",
    "replace with",
    "insert here",
];

const GENERATE_EXACT_PLACEHOLDER_PHRASES: &[&str] = &[
    "feature name",
    "workflow name",
    "service name",
    "entity name",
    "field1",
    "field 1",
    "field2",
    "field 2",
];

const ANGLE_PLACEHOLDER_KEYWORDS: &[&str] = &[
    "project", "feature", "workflow", "service", "entity", "field", "value", "token", "email",
    "password", "id", "name", "type", "path", "slug", "section",
];

fn normalize_quality_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_literal_placeholder_terms(text: &str) -> bool {
    let normalized = normalize_quality_text(text);
    !normalized.is_empty()
        && (EXACT_PLACEHOLDER_PHRASES
            .iter()
            .any(|phrase| normalized == *phrase)
            || SUBSTRING_PLACEHOLDER_PHRASES
                .iter()
                .any(|phrase| normalized.contains(phrase)))
}

fn contains_named_angle_placeholder(text: &str) -> bool {
    let mut search_from = 0;
    while let Some(open_offset) = text[search_from..].find('<') {
        let open = search_from + open_offset;
        let Some(close_offset) = text[open + 1..].find('>') else {
            break;
        };
        let close = open + 1 + close_offset;
        if is_named_angle_placeholder(&text[open + 1..close]) {
            return true;
        }
        search_from = close + 1;
    }

    false
}

fn is_named_angle_placeholder(candidate: &str) -> bool {
    let trimmed = candidate.trim();
    if trimmed.is_empty() || trimmed.len() > 40 {
        return false;
    }

    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' '))
    {
        return false;
    }

    let normalized = normalize_quality_text(trimmed);
    if normalized.is_empty() {
        return false;
    }

    trimmed.contains('-')
        || trimmed.contains('_')
        || ANGLE_PLACEHOLDER_KEYWORDS
            .iter()
            .any(|keyword| normalized == *keyword || normalized.contains(keyword))
}

fn has_placeholder_language(text: &str) -> bool {
    if contains_named_angle_placeholder(text) {
        return true;
    }

    has_literal_placeholder_terms(text)
}

fn has_generate_placeholder_language(text: &str) -> bool {
    let normalized = normalize_quality_text(text);
    if normalized.is_empty() {
        return false;
    }

    GENERATE_EXACT_PLACEHOLDER_PHRASES
        .iter()
        .any(|phrase| normalized == *phrase)
        || SUBSTRING_PLACEHOLDER_PHRASES
            .iter()
            .any(|phrase| normalized.contains(phrase))
        || (normalized.split_whitespace().count() <= 8
            && matches!(normalized.as_str(), "placeholder" | "todo" | "tbd"))
}

fn is_generic_project_name(name: &str) -> bool {
    let normalized = normalize_quality_text(name);
    if normalized.is_empty() {
        return true;
    }

    if matches!(
        normalized.as_str(),
        "unknown"
            | "project"
            | "app"
            | "application"
            | "sample project"
            | "sample app"
            | "new project"
            | "my project"
            | "untitled project"
            | "test project"
            | "mvp project"
    ) {
        return true;
    }

    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    !tokens.is_empty()
        && tokens.len() <= 3
        && tokens.iter().all(|token| {
            matches!(
                *token,
                "unknown"
                    | "project"
                    | "app"
                    | "application"
                    | "sample"
                    | "new"
                    | "my"
                    | "untitled"
                    | "test"
                    | "mvp"
                    | "demo"
                    | "prototype"
                    | "tool"
            )
        })
}

fn push_quality_issue_for_text(
    issues: &mut Vec<String>,
    label: &str,
    text: &str,
    min_words: usize,
) {
    if text.trim().is_empty() {
        issues.push(format!("`{label}` is empty."));
        return;
    }
    if has_placeholder_language(text) {
        issues.push(format!(
            "`{label}` still contains placeholder language: {}",
            truncate_for_prompt(text, 120)
        ));
        return;
    }
    if text.split_whitespace().count() < min_words {
        issues.push(format!(
            "`{label}` is too short to guide implementation. Make it more concrete."
        ));
    }
}

fn collect_phase_quality_issues(phase: WizardPhase, value: &JsonValue) -> Vec<String> {
    let mut issues = Vec::new();

    match phase {
        WizardPhase::Vision => {
            let project_name = value
                .get("project_name")
                .and_then(|item| item.as_str())
                .unwrap_or("");
            if is_generic_project_name(project_name) {
                issues.push(
                    "`project_name` is generic. Use a concrete product name tied to the domain."
                        .to_string(),
                );
            }

            push_quality_issue_for_text(
                &mut issues,
                "elevator_pitch",
                value
                    .get("elevator_pitch")
                    .and_then(|item| item.as_str())
                    .unwrap_or(""),
                5,
            );
            push_quality_issue_for_text(
                &mut issues,
                "problem_statement",
                value
                    .get("problem_statement")
                    .and_then(|item| item.as_str())
                    .unwrap_or(""),
                8,
            );

            for field in ["target_users", "success_criteria", "scope_exclusions"] {
                if let Some(items) = value.get(field).and_then(|item| item.as_array()) {
                    if items.is_empty() {
                        issues.push(format!(
                            "`{field}` must contain at least one concrete item."
                        ));
                    }
                    for entry in items.iter().take(4) {
                        if let Some(text) = entry.as_str() {
                            if has_placeholder_language(text) {
                                issues.push(format!(
                                    "`{field}` contains placeholder language: {}",
                                    truncate_for_prompt(text, 120)
                                ));
                                break;
                            }
                        }
                    }
                } else {
                    issues.push(format!("`{field}` must be an array of concrete items."));
                }
            }
        }
        WizardPhase::Functionality => {
            for (field, label) in [
                ("mvp_features", "mvp_features"),
                ("deferred_features", "deferred_features"),
            ] {
                if let Some(items) = value.get(field).and_then(|item| item.as_array()) {
                    for (index, entry) in items.iter().enumerate().take(4) {
                        push_quality_issue_for_text(
                            &mut issues,
                            &format!("{label}[{index}].name"),
                            entry
                                .get("name")
                                .and_then(|item| item.as_str())
                                .unwrap_or(""),
                            2,
                        );
                        push_quality_issue_for_text(
                            &mut issues,
                            &format!("{label}[{index}].description"),
                            entry
                                .get("description")
                                .and_then(|item| item.as_str())
                                .unwrap_or(""),
                            5,
                        );
                    }
                }
            }

            if let Some(workflows) = value.get("user_workflows").and_then(|item| item.as_array()) {
                for (index, workflow) in workflows.iter().enumerate().take(4) {
                    push_quality_issue_for_text(
                        &mut issues,
                        &format!("user_workflows[{index}].name"),
                        workflow
                            .get("name")
                            .and_then(|item| item.as_str())
                            .unwrap_or(""),
                        2,
                    );
                    if let Some(steps) = workflow.get("steps").and_then(|item| item.as_array()) {
                        if steps.len() < 2 {
                            issues.push(format!(
                                "`user_workflows[{index}].steps` should describe at least two concrete steps."
                            ));
                        }
                        for step in steps.iter().take(4) {
                            if let Some(text) = step.as_str() {
                                if has_placeholder_language(text) {
                                    issues.push(format!(
                                        "`user_workflows[{index}].steps` contains placeholder language: {}",
                                        truncate_for_prompt(text, 120)
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        WizardPhase::Technical => {
            if let Some(entities) = value.get("data_model").and_then(|item| item.as_array()) {
                for (index, entity) in entities.iter().enumerate().take(4) {
                    push_quality_issue_for_text(
                        &mut issues,
                        &format!("data_model[{index}].entity"),
                        entity
                            .get("entity")
                            .and_then(|item| item.as_str())
                            .unwrap_or(""),
                        1,
                    );
                    if let Some(fields) = entity.get("fields").and_then(|item| item.as_array()) {
                        for field in fields.iter().take(4) {
                            if let Some(text) = field.as_str() {
                                if has_placeholder_language(text) {
                                    issues.push(format!(
                                        "`data_model[{index}].fields` contains placeholder language: {}",
                                        truncate_for_prompt(text, 120)
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                    if let Some(relationships) =
                        entity.get("relationships").and_then(|item| item.as_array())
                    {
                        for relationship in relationships.iter().take(4) {
                            if let Some(text) = relationship.as_str() {
                                if has_placeholder_language(text) {
                                    issues.push(format!(
                                        "`data_model[{index}].relationships` contains placeholder language: {}",
                                        truncate_for_prompt(text, 120)
                                    ));
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if let Some(integrations) = value.get("integrations").and_then(|item| item.as_array()) {
                for (index, integration) in integrations.iter().enumerate().take(4) {
                    push_quality_issue_for_text(
                        &mut issues,
                        &format!("integrations[{index}].service"),
                        integration
                            .get("service")
                            .and_then(|item| item.as_str())
                            .unwrap_or(""),
                        1,
                    );
                    push_quality_issue_for_text(
                        &mut issues,
                        &format!("integrations[{index}].purpose"),
                        integration
                            .get("purpose")
                            .and_then(|item| item.as_str())
                            .unwrap_or(""),
                        4,
                    );
                }
            }

            for field in ["constraints", "performance_requirements"] {
                if let Some(items) = value.get(field).and_then(|item| item.as_array()) {
                    for entry in items.iter().take(4) {
                        if let Some(text) = entry.as_str() {
                            if has_placeholder_language(text) {
                                issues.push(format!(
                                    "`{field}` contains placeholder language: {}",
                                    truncate_for_prompt(text, 120)
                                ));
                                break;
                            }
                        }
                    }
                }
            }
        }
        WizardPhase::Generate => {
            if let Some(sections) = value.get("sections").and_then(|item| item.as_array()) {
                if sections.is_empty() {
                    issues.push("`sections` must contain generated PRD content.".to_string());
                }
                for (index, section) in sections.iter().enumerate().take(4) {
                    let content = section
                        .get("content")
                        .and_then(|item| item.as_str())
                        .unwrap_or("");
                    if content.trim().is_empty() {
                        issues.push(format!("`sections[{index}].content` is empty."));
                    } else if has_generate_placeholder_language(content) {
                        issues.push(format!(
                            "`sections[{index}].content` still contains placeholder language: {}",
                            truncate_for_prompt(content, 120)
                        ));
                    } else if content.split_whitespace().count() < 4 {
                        issues.push(format!(
                            "`sections[{index}].content` is too short to guide implementation. Make it more concrete."
                        ));
                    }
                }
            } else {
                issues.push("`sections` must be an array of generated PRD sections.".to_string());
            }
        }
        WizardPhase::TaskReview => {
            if let Some(tasks) = value.get("tasks").and_then(|item| item.as_array()) {
                if tasks.is_empty() {
                    issues.push("`tasks` must contain a concrete reviewed task list.".to_string());
                }
                for (index, task) in tasks.iter().enumerate().take(6) {
                    let description = task
                        .get("description")
                        .and_then(|item| item.as_str())
                        .unwrap_or("");
                    if has_placeholder_language(description) {
                        issues.push(format!(
                            "`tasks[{index}].description` still contains placeholder language: {}",
                            truncate_for_prompt(description, 120)
                        ));
                    }
                    let rationale = task
                        .get("rationale")
                        .and_then(|item| item.as_str())
                        .unwrap_or("");
                    if has_placeholder_language(rationale) {
                        issues.push(format!(
                            "`tasks[{index}].rationale` still contains placeholder language: {}",
                            truncate_for_prompt(rationale, 120)
                        ));
                    }
                    match task
                        .get("acceptance_criteria")
                        .and_then(|item| item.as_array())
                    {
                        Some(criteria) if !criteria.is_empty() => {
                            for criterion in criteria.iter().take(3) {
                                if let Some(text) = criterion.as_str() {
                                    if has_placeholder_language(text) {
                                        issues.push(format!(
                                            "`tasks[{index}].acceptance_criteria` still contains placeholder language: {}",
                                            truncate_for_prompt(text, 120)
                                        ));
                                        break;
                                    }
                                }
                            }
                        }
                        _ => issues.push(format!(
                            "`tasks[{index}].acceptance_criteria` must contain at least one concrete acceptance check."
                        )),
                    }
                }
            }
        }
        WizardPhase::BuildTestConfig => {
            if let Some(test_tasks) = value.get("test_tasks").and_then(|item| item.as_array()) {
                for (index, task) in test_tasks.iter().enumerate().take(6) {
                    let description = task
                        .get("description")
                        .and_then(|item| item.as_str())
                        .unwrap_or("");
                    if has_placeholder_language(description) {
                        issues.push(format!(
                            "`test_tasks[{index}].description` still contains placeholder language: {}",
                            truncate_for_prompt(description, 120)
                        ));
                    }
                }
            }
        }
        WizardPhase::GapAnalysis | WizardPhase::IterationMode | WizardPhase::Launch => {}
    }

    issues.truncate(8);
    issues
}

fn should_enforce_phase_quality(provider_name: &str) -> bool {
    provider_name == "copilot"
}

fn build_project_summary_context(gathered_info: &JsonValue) -> String {
    let mut lines = Vec::new();

    if let Some(vision) = gathered_info.get("vision") {
        if let Some(name) = vision.get("project_name").and_then(|v| v.as_str()) {
            if !is_generic_project_name(name) {
                lines.push(format!("- Project: {}", name));
            }
        }
        if let Some(problem) = vision.get("problem_statement").and_then(|v| v.as_str()) {
            lines.push(format!("- Problem: {}", truncate_for_prompt(problem, 220)));
        }
        if let Some(users) = vision.get("target_users").and_then(|v| v.as_array()) {
            let names: Vec<&str> = users.iter().filter_map(|value| value.as_str()).collect();
            if !names.is_empty() {
                lines.push(format!("- Target users: {}", names.join(", ")));
            }
        }
        if let Some(criteria) = vision.get("success_criteria").and_then(|v| v.as_array()) {
            let items: Vec<&str> = criteria
                .iter()
                .filter_map(|value| value.as_str())
                .take(4)
                .collect();
            if !items.is_empty() {
                lines.push(format!("- Success criteria: {}", items.join("; ")));
            }
        }
        if let Some(exclusions) = vision.get("scope_exclusions").and_then(|v| v.as_array()) {
            let items: Vec<&str> = exclusions
                .iter()
                .filter_map(|value| value.as_str())
                .take(4)
                .collect();
            if !items.is_empty() {
                lines.push(format!("- Scope exclusions: {}", items.join("; ")));
            }
        }
    }

    if let Some(functionality) = gathered_info.get("functionality") {
        if let Some(features) = functionality.get("mvp_features").and_then(|v| v.as_array()) {
            let names: Vec<&str> = features
                .iter()
                .filter_map(|feature| feature.get("name").and_then(|v| v.as_str()))
                .take(6)
                .collect();
            if !names.is_empty() {
                lines.push(format!("- MVP features: {}", names.join(", ")));
            }
        }
    }

    if let Some(technical) = gathered_info.get("technical") {
        if let Some(platform) = technical.get("platform") {
            let languages = platform
                .get("languages")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let frameworks = platform
                .get("frameworks")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let database = platform
                .get("database")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let hosting = platform
                .get("hosting")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let mut platform_parts = Vec::new();
            if !languages.is_empty() {
                platform_parts.push(format!("languages: {}", languages));
            }
            if !frameworks.is_empty() {
                platform_parts.push(format!("frameworks: {}", frameworks));
            }
            if !database.is_empty() {
                platform_parts.push(format!("database: {}", database));
            }
            if !hosting.is_empty() {
                platform_parts.push(format!("hosting: {}", hosting));
            }

            if !platform_parts.is_empty() {
                lines.push(format!("- Platform: {}", platform_parts.join("; ")));
            }
        }
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("\n## Project Summary\n{}\n", lines.join("\n"))
    }
}

fn build_source_requirements_context(gathered_info: &JsonValue) -> String {
    let Some(source) = gathered_info.get("source_requirements") else {
        return String::new();
    };

    let mut sections = Vec::new();
    for (label, key) in [
        ("Goal", "goal"),
        ("Repository Context", "repository_context"),
        ("Requested Outcome", "requested_outcome"),
        ("Hard Constraints", "hard_constraints"),
        ("Acceptance Checks", "acceptance_checks"),
        ("Source Excerpt", "source_excerpt"),
    ] {
        if let Some(text) = source.get(key).and_then(|value| value.as_str()) {
            sections.push(format!("{label}:\n{text}"));
        }
    }

    if sections.is_empty() {
        String::new()
    } else {
        format!(
            "\n## Source Requirements (authoritative)\n{}\n",
            sections.join("\n\n")
        )
    }
}

fn build_generated_section_outline(gathered_info: &JsonValue) -> String {
    let Some(sections) = gathered_info
        .get("generate")
        .and_then(|value| value.get("sections"))
        .and_then(|value| value.as_array())
    else {
        return String::new();
    };

    let lines: Vec<String> = sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let title = section
                .get("title")
                .and_then(|value| value.as_str())
                .unwrap_or("Untitled");
            let content = section
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            format!(
                "- {} {}: {}",
                index + 1,
                title,
                truncate_for_prompt(content, 180)
            )
        })
        .collect();

    if lines.is_empty() {
        String::new()
    } else {
        format!("\n## PRD Section Outline\n{}\n", lines.join("\n"))
    }
}

fn build_functionality_detail_context(gathered_info: &JsonValue) -> String {
    let Some(functionality) = gathered_info.get("functionality") else {
        return String::new();
    };

    let mut lines = Vec::new();

    if let Some(features) = functionality
        .get("mvp_features")
        .and_then(|value| value.as_array())
    {
        for feature in features.iter().take(6) {
            let name = feature
                .get("name")
                .and_then(|value| value.as_str())
                .or_else(|| feature.as_str())
                .unwrap_or("Unnamed feature");
            let description = feature
                .get("description")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let priority = feature
                .get("priority")
                .and_then(|value| value.as_i64())
                .map(|value| format!("P{} ", value))
                .unwrap_or_default();

            if description.is_empty() {
                lines.push(format!("- {}{}", priority, name));
            } else {
                lines.push(format!(
                    "- {}{}: {}",
                    priority,
                    name,
                    truncate_for_prompt(description, 140)
                ));
            }
        }

        if features.len() > 6 {
            lines.push(format!("- ... {} more MVP features", features.len() - 6));
        }
    }

    if let Some(workflows) = functionality
        .get("user_workflows")
        .and_then(|value| value.as_array())
    {
        for workflow in workflows.iter().take(4) {
            let name = workflow
                .get("name")
                .and_then(|value| value.as_str())
                .or_else(|| workflow.as_str())
                .unwrap_or("Unnamed workflow");
            let steps = workflow
                .get("steps")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str())
                        .take(3)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            if steps.is_empty() {
                lines.push(format!("- Workflow: {}", name));
            } else {
                lines.push(format!("- Workflow: {} ({})", name, steps.join(" -> ")));
            }
        }
    }

    if let Some(deferred) = functionality
        .get("deferred_features")
        .and_then(|value| value.as_array())
    {
        let names: Vec<&str> = deferred
            .iter()
            .filter_map(|feature| {
                feature
                    .get("name")
                    .and_then(|value| value.as_str())
                    .or_else(|| feature.as_str())
            })
            .take(4)
            .collect();
        if !names.is_empty() {
            lines.push(format!("- Deferred for later: {}", names.join(", ")));
        }
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("\n## Functionality Details\n{}\n", lines.join("\n"))
    }
}

fn build_technical_detail_context(gathered_info: &JsonValue) -> String {
    let Some(technical) = gathered_info.get("technical") else {
        return String::new();
    };

    let mut lines = Vec::new();

    let platform = technical.get("platform").unwrap_or(technical);
    let languages = platform
        .get("languages")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let frameworks = platform
        .get("frameworks")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let database = platform
        .get("database")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let hosting = platform
        .get("hosting")
        .and_then(|value| value.as_str())
        .or_else(|| technical.get("platform").and_then(|value| value.as_str()))
        .unwrap_or("");

    let mut platform_parts = Vec::new();
    if !languages.is_empty() {
        platform_parts.push(format!("languages: {}", languages));
    }
    if !frameworks.is_empty() {
        platform_parts.push(format!("frameworks: {}", frameworks));
    }
    if !database.is_empty() {
        platform_parts.push(format!("database: {}", database));
    }
    if !hosting.is_empty() {
        platform_parts.push(format!("hosting: {}", hosting));
    }
    if !platform_parts.is_empty() {
        lines.push(format!("- Platform: {}", platform_parts.join("; ")));
    }

    if let Some(entities) = technical
        .get("data_model")
        .and_then(|value| value.as_array())
    {
        for entity in entities.iter().take(5) {
            let name = entity
                .get("entity")
                .and_then(|value| value.as_str())
                .unwrap_or("Unnamed entity");
            let field_count = entity
                .get("fields")
                .and_then(|value| value.as_array())
                .map(|items| items.len())
                .unwrap_or(0);
            if field_count > 0 {
                lines.push(format!("- Data model: {} ({} fields)", name, field_count));
            } else {
                lines.push(format!("- Data model: {}", name));
            }
        }
    }

    if let Some(integrations) = technical
        .get("integrations")
        .and_then(|value| value.as_array())
    {
        for integration in integrations.iter().take(5) {
            let name = integration
                .get("service")
                .and_then(|value| value.as_str())
                .or_else(|| integration.as_str())
                .unwrap_or("Unnamed integration");
            let purpose = integration
                .get("purpose")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if purpose.is_empty() {
                lines.push(format!("- Integration: {}", name));
            } else {
                lines.push(format!(
                    "- Integration: {} ({})",
                    name,
                    truncate_for_prompt(purpose, 120)
                ));
            }
        }
    }

    if let Some(constraints) = technical
        .get("constraints")
        .and_then(|value| value.as_array())
    {
        let items: Vec<&str> = constraints
            .iter()
            .filter_map(|value| value.as_str())
            .take(5)
            .collect();
        if !items.is_empty() {
            lines.push(format!("- Constraints: {}", items.join("; ")));
        }
    }

    if let Some(requirements) = technical
        .get("performance_requirements")
        .and_then(|value| value.as_array())
    {
        let items: Vec<&str> = requirements
            .iter()
            .filter_map(|value| value.as_str())
            .take(4)
            .collect();
        if !items.is_empty() {
            lines.push(format!("- Performance: {}", items.join("; ")));
        }
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("\n## Technical Details Summary\n{}\n", lines.join("\n"))
    }
}

fn build_gap_analysis_detail_context(gathered_info: &JsonValue) -> String {
    let Some(gap_analysis) = gathered_info.get("gap_analysis") else {
        return String::new();
    };

    let mut lines = Vec::new();

    if let Some(gaps) = gap_analysis.get("gaps").and_then(|value| value.as_array()) {
        for gap in gaps.iter().take(6) {
            let area = gap
                .get("area")
                .and_then(|value| value.as_str())
                .unwrap_or("Unspecified area");
            let issue = gap
                .get("issue")
                .or_else(|| gap.get("description"))
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let suggestion = gap
                .get("suggestion")
                .and_then(|value| value.as_str())
                .unwrap_or("");

            let mut detail = truncate_for_prompt(issue, 140);
            if !suggestion.is_empty() {
                detail.push_str(&format!(
                    " | Suggested fix: {}",
                    truncate_for_prompt(suggestion, 100)
                ));
            }
            lines.push(format!("- Gap: {} -> {}", area, detail));
        }
    }

    if let Some(contradictions) = gap_analysis
        .get("contradictions")
        .and_then(|value| value.as_array())
    {
        for contradiction in contradictions.iter().take(4) {
            let between = contradiction
                .get("between")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|value| value.as_str())
                        .collect::<Vec<_>>()
                        .join(" vs ")
                })
                .unwrap_or_else(|| "unknown sections".to_string());
            let issue = contradiction
                .get("issue")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            lines.push(format!(
                "- Contradiction: {} -> {}",
                between,
                truncate_for_prompt(issue, 120)
            ));
        }
    }

    if let Some(recommendations) = gap_analysis
        .get("recommendations")
        .and_then(|value| value.as_array())
    {
        for recommendation in recommendations.iter().take(5) {
            let text = recommendation
                .get("recommendation")
                .or_else(|| recommendation.get("topic"))
                .and_then(|value| value.as_str())
                .or_else(|| recommendation.as_str())
                .unwrap_or("");
            if !text.is_empty() {
                lines.push(format!(
                    "- Recommendation: {}",
                    truncate_for_prompt(text, 140)
                ));
            }
        }
    }

    if lines.is_empty() {
        String::new()
    } else {
        format!("\n## Gap Analysis Summary\n{}\n", lines.join("\n"))
    }
}

fn phase_context_for_prompt(phase: WizardPhase, gathered_info: &JsonValue) -> String {
    let source_requirements = build_source_requirements_context(gathered_info);
    let phase_specific = match phase {
        WizardPhase::Vision => String::new(),
        WizardPhase::Functionality => selected_gathered_context(
            gathered_info,
            &[("vision", "Vision & Problem (from Phase 1)")],
        ),
        WizardPhase::Technical => selected_gathered_context(
            gathered_info,
            &[
                ("vision", "Vision & Problem (from Phase 1)"),
                ("functionality", "Functionality (from Phase 2)"),
            ],
        ),
        WizardPhase::GapAnalysis => format!(
            "{}{}{}",
            build_project_summary_context(gathered_info),
            build_functionality_detail_context(gathered_info),
            build_technical_detail_context(gathered_info),
        ),
        WizardPhase::Generate => format!(
            "{}{}{}{}",
            build_project_summary_context(gathered_info),
            build_functionality_detail_context(gathered_info),
            build_technical_detail_context(gathered_info),
            build_gap_analysis_detail_context(gathered_info),
        ),
        _ => String::new(),
    };

    let context = format!("{source_requirements}{phase_specific}");

    if context.is_empty() {
        String::new()
    } else {
        format!("\n## Previously Gathered Information\n{context}")
    }
}

fn wizard_phase_timeout_secs(phase: WizardPhase) -> u64 {
    match phase {
        WizardPhase::GapAnalysis | WizardPhase::TaskReview | WizardPhase::BuildTestConfig => 300,
        WizardPhase::Launch => 30,
        _ => 180,
    }
}

fn emit_prompt_diagnostics(
    event_sink: Option<&WizardEventSink>,
    provider: &dyn Provider,
    phase: WizardPhase,
    prompt: &str,
    timeout_secs: u64,
) {
    let estimated_tokens = budget::estimate_tokens(prompt);
    emit_wizard_event(
        event_sink,
        Event::Info(format!(
            "Wizard phase {} using {}: {} chars (~{} tokens), {}s timeout",
            phase as i32,
            provider.name(),
            prompt.len(),
            estimated_tokens,
            timeout_secs
        )),
    );

    if estimated_tokens > 3500 {
        emit_wizard_event(
            event_sink,
            Event::Warning(format!(
                "Wizard phase {} prompt is large (~{} tokens). Expect slower backend responses.",
                phase as i32, estimated_tokens
            )),
        );
    }
}

/// Summary of task sizing analysis performed during Phase 6.
#[derive(Debug, Clone, Default)]
pub struct SizingSummary {
    pub small: usize,
    pub medium: usize,
    pub large: usize,
    pub xl: usize,
    pub total_splits: usize,
    pub total_rewrites: usize,
    pub total_merges: usize,
}

/// A task that was split into smaller sub-tasks.
#[derive(Debug, Clone)]
pub struct TaskSplitRecord {
    pub original: String,
    pub into: Vec<String>,
    pub reason: String,
}

/// A task description that was rewritten to be more concrete.
#[derive(Debug, Clone)]
pub struct TaskRewriteRecord {
    pub original: String,
    pub rewritten: String,
    pub reason: String,
}

/// Tasks that were merged into a single task.
#[derive(Debug, Clone)]
pub struct TaskMergeRecord {
    pub merged: Vec<String>,
    pub into: String,
    pub reason: String,
}

/// A test task generated by Phase 7 test strategy analysis.
#[derive(Debug, Clone)]
pub struct TestTaskRecord {
    pub description: String,
    pub depends_on_feature: usize,
    pub covers_features: Vec<usize>,
    pub target_files: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Phase6TaskPlanRecord {
    task_kind: String,
    feature_group: Option<String>,
    coverage_mode: String,
}

/// Save wizard state to the database (upsert).
pub fn save_wizard_state(conn: &Connection, state: &mut WizardState) -> Result<()> {
    let completed_json =
        serde_json::to_string(&state.completed_phases).unwrap_or_else(|_| "[]".to_string());
    let info_json =
        serde_json::to_string(&state.gathered_info).unwrap_or_else(|_| "{}".to_string());

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
        state.id = conn.last_insert_rowid();
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

    let prior_context = phase_context_for_prompt(phase, &state.gathered_info);

    let doc_context = existing_doc
        .map(|doc| format!("\n## Existing Document\n{}\n", doc))
        .unwrap_or_default();

    let source_rules = if existing_doc.is_some() {
        r#"
## Source-of-Truth Rules
- Treat the existing document as the authoritative source of scope and acceptance.
- Preserve exact literal values from it, including enum or status names, commands, file paths, file names, and acceptance checks.
- Do not invent alternate contracts, validations, or extra scope that the source document does not request.
- If the source document requests a specific backlog size or implementation shape, keep later phases aligned to it.
"#
    } else {
        ""
    };

    match phase {
        WizardPhase::Vision => format!(
            r#"You are helping create a Product Requirements Document (PRD).

Phase 1: Vision & Problem

{template_context}{prior_context}{doc_context}{source_rules}

Rules:
- `project_name` must be a concrete product name tied to the domain. If no name exists yet, invent one.
- Do not use generic names like `unknown`, `project`, `app`, `sample project`, or `my project`.
- Every array entry must be concrete and specific to this project domain. No placeholders or filler text.
- `success_criteria` should be measurable outcomes, not generic aspirations.
- `scope_exclusions` should name real things the MVP will not do.

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

{template_context}{prior_context}{doc_context}{source_rules}

Based on the vision, define the features in JSON format:
- Reuse the concrete domain nouns from phase 1.
- Do not use placeholders like `feature name`, `workflow name`, `some user`, or `TBD`.
- Every MVP feature should describe a real user-visible capability for this specific product.
- Every workflow should describe concrete user or system actions for this product.
- If the existing document names exact literals such as statuses, commands, file paths, or acceptance checks, carry those exact literals forward.
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

{template_context}{prior_context}{doc_context}{source_rules}

Define the technical architecture in JSON format:
- Use concrete domain entities, field names, relationships, and integrations from the prior phases.
- Do not use placeholders like `field1: type`, `second entity`, `service name`, `constraint 1`, or `requirement 1`.
- If there are no external integrations, return an empty `integrations` array instead of placeholder entries.
- Preserve exact constraints, commands, file paths, and platform expectations from the existing document when they are explicitly provided.
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

{template_context}{prior_context}{doc_context}{source_rules}

Review everything gathered so far and identify:
1. Missing details that would block implementation
2. Contradictions between sections
3. Ambiguous requirements that need clarification
4. Edge cases not covered
5. Security or performance concerns not addressed

## SPECIFICITY CHECK

Review each section of the gathered information for vague language. Flag any section that uses:
- Vague qualifiers: 'should', 'might', 'could', 'may', 'possibly', 'generally'
- Placeholder terms: 'etc.', 'various', 'some', 'appropriate', 'as needed', 'TBD'
- Missing specifics: no concrete inputs/outputs, no acceptance criteria, no measurable behavior

For each section, rate it as:
- SPECIFIC: Has concrete acceptance criteria, measurable outcomes, defined inputs/outputs
- NEEDS_DETAIL: Has some concrete details but lacks full acceptance criteria
- VAGUE: Uses vague language with no concrete acceptance criteria

Do not proceed to Phase 5 with any VAGUE sections. Rewrite them now with specific acceptance criteria.
Keep the response concise:
- Return at most 8 gaps, 4 contradictions, 6 recommendations, 8 section_ratings, and 4 rewritten_sections
- Keep each string concise and plain
- In rewritten_sections, use plain prose only. Do not include code blocks, JSON examples, or quoted example payloads.

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
  ],
  "section_ratings": [
    {{"section": "section name", "rating": "SPECIFIC or NEEDS_DETAIL or VAGUE", "issues": ["vague language found"]}}
  ],
  "rewritten_sections": [
    {{"section": "section name", "rewritten": "concise rewrite guidance with concrete acceptance criteria"}}
  ]
}}

Respond ONLY with valid JSON."#
        ),
        WizardPhase::Generate => format!(
            r#"You are generating a structured PRD from gathered information.

{template_context}{prior_context}{doc_context}{source_rules}

Generate the complete PRD content as a JSON object where each key is a section title
from the template and each value is the markdown content for that section.
Include all relevant information gathered from prior phases.
Keep the section content plain and parse-safe:
- Use normal markdown paragraphs and bullet lists only
- Do not include JSON snippets or code fences inside section content
- Avoid double-quoted literal examples inside section content; use backticks for enum values, field names, and literal strings
- Use the same concrete project name and domain terms established in earlier phases. Do not reintroduce placeholders.

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
        WizardPhase::BuildTestConfig => {
            // Phase 7 uses build_build_test_config_prompt() for the full prompt
            // when technical details and tasks are available. This fallback uses
            // only gathered_info with an empty task list.
            build_build_test_config_prompt(&state.gathered_info, &[])
        }
        WizardPhase::IterationMode => {
            // Phase 8 uses build_iteration_mode_prompt() for the full prompt
            // when project context and task count are available. This fallback
            // uses only gathered_info with a zero task count.
            build_iteration_mode_prompt(&state.gathered_info, 0)
        }
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
    if maybe_store_source_requirements(state, from_doc) {
        save_wizard_state(prd_conn, state)?;
    }

    let phases = [
        WizardPhase::Vision,
        WizardPhase::Functionality,
        WizardPhase::Technical,
    ];

    for phase in &phases {
        // Skip already completed phases (for resume)
        if state.completed_phases.contains(&(*phase as i32)) {
            continue;
        }

        state.current_phase = *phase;
        save_wizard_state(prd_conn, state)?;

        let prompt = build_phase_prompt(*phase, state, from_doc);
        let response = execute_wizard_prompt(provider, *phase, &prompt, None).await?;

        // Parse JSON response
        let data = parse_json_response(&response, provider, *phase, &prompt, None).await?;
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
        return Err(DialError::UserError(format!(
            "File not found: {}",
            from_path
        )));
    }
    let content = std::fs::read_to_string(path)?;
    Ok(content)
}

/// Execute a wizard prompt against the provider.
async fn execute_wizard_prompt(
    provider: &dyn Provider,
    phase: WizardPhase,
    prompt: &str,
    event_sink: Option<&WizardEventSink>,
) -> Result<String> {
    execute_wizard_prompt_with_heartbeat(
        provider,
        phase,
        prompt,
        event_sink,
        Duration::from_secs(15),
        Duration::from_secs(30),
    )
    .await
}

async fn execute_wizard_prompt_with_heartbeat(
    provider: &dyn Provider,
    phase: WizardPhase,
    prompt: &str,
    event_sink: Option<&WizardEventSink>,
    first_heartbeat_after: Duration,
    heartbeat_every: Duration,
) -> Result<String> {
    let timeout_secs = wizard_phase_timeout_secs(phase);
    emit_prompt_diagnostics(event_sink, provider, phase, prompt, timeout_secs);

    let request = ProviderRequest {
        prompt: prompt.to_string(),
        work_dir: wizard_work_dir(),
        output_schema: wizard_phase_output_schema(phase),
        max_tokens: Some(4096),
        model: None,
        timeout_secs: Some(timeout_secs),
    };

    let started = Instant::now();
    let execute = provider.execute(request);
    tokio::pin!(execute);

    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + first_heartbeat_after,
        heartbeat_every,
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let response = loop {
        tokio::select! {
            result = &mut execute => break result?,
            _ = heartbeat.tick(), if phase != WizardPhase::Launch => {
                emit_wizard_event(
                    event_sink,
                    Event::WizardHeartbeat {
                        phase: phase as u8,
                        name: phase.name().to_string(),
                        backend: provider.name().to_string(),
                        elapsed_secs: started.elapsed().as_secs(),
                    },
                );
            }
        }
    };
    let duration_secs = response
        .duration_secs
        .unwrap_or_else(|| started.elapsed().as_secs_f64());

    emit_wizard_event(
        event_sink,
        Event::Info(format!(
            "Wizard phase {} backend response received in {:.1}s ({} chars)",
            phase as i32,
            duration_secs,
            response.output.len()
        )),
    );

    if !response.success {
        return Err(DialError::WizardError(format!(
            "Provider returned failure: {}",
            response.output
        )));
    }

    Ok(response.output)
}

fn wizard_work_dir() -> String {
    // Run wizard prompts from a neutral temp directory so agentic CLI backends
    // do not inherit project-local instructions (for example AGENTS.md).
    std::env::temp_dir().to_string_lossy().to_string()
}

fn wizard_phase_output_schema(phase: WizardPhase) -> Option<String> {
    Some(
        match phase {
            WizardPhase::Vision => serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "project_name",
                    "elevator_pitch",
                    "problem_statement",
                    "target_users",
                    "success_criteria",
                    "scope_exclusions"
                ],
                "properties": {
                    "project_name": { "type": "string", "minLength": 3 },
                    "elevator_pitch": { "type": "string", "minLength": 20 },
                    "problem_statement": { "type": "string", "minLength": 30 },
                    "target_users": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string", "minLength": 3 }
                    },
                    "success_criteria": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string", "minLength": 8 }
                    },
                    "scope_exclusions": {
                        "type": "array",
                        "minItems": 1,
                        "items": { "type": "string", "minLength": 5 }
                    }
                }
            }),
            WizardPhase::Functionality => serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["mvp_features", "deferred_features", "user_workflows"],
                "properties": {
                    "mvp_features": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["name", "description", "priority"],
                            "properties": {
                                "name": { "type": "string", "minLength": 3 },
                                "description": { "type": "string", "minLength": 12 },
                                "priority": { "type": "integer" }
                            }
                        }
                    },
                    "deferred_features": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["name", "description", "rationale"],
                            "properties": {
                                "name": { "type": "string", "minLength": 3 },
                                "description": { "type": "string", "minLength": 12 },
                                "rationale": { "type": "string", "minLength": 8 }
                            }
                        }
                    },
                    "user_workflows": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["name", "steps"],
                            "properties": {
                                "name": { "type": "string", "minLength": 3 },
                                "steps": {
                                    "type": "array",
                                    "minItems": 2,
                                    "items": { "type": "string", "minLength": 4 }
                                }
                            }
                        }
                    }
                }
            }),
            WizardPhase::Technical => serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "data_model",
                    "integrations",
                    "platform",
                    "constraints",
                    "performance_requirements"
                ],
                "properties": {
                    "data_model": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["entity", "fields", "relationships"],
                            "properties": {
                                "entity": { "type": "string", "minLength": 2 },
                                "fields": {
                                    "type": "array",
                                    "minItems": 1,
                                    "items": { "type": "string", "minLength": 4 }
                                },
                                "relationships": {
                                    "type": "array",
                                    "items": { "type": "string", "minLength": 4 }
                                }
                            }
                        }
                    },
                    "integrations": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["service", "purpose", "api_type"],
                            "properties": {
                                "service": { "type": "string", "minLength": 2 },
                                "purpose": { "type": "string", "minLength": 8 },
                                "api_type": { "type": "string", "minLength": 2 }
                            }
                        }
                    },
                    "platform": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["languages", "frameworks", "database", "hosting"],
                        "properties": {
                            "languages": {
                                "type": "array",
                                "minItems": 1,
                                "items": { "type": "string", "minLength": 2 }
                            },
                            "frameworks": {
                                "type": "array",
                                "items": { "type": "string", "minLength": 2 }
                            },
                            "database": { "type": "string", "minLength": 2 },
                            "hosting": { "type": "string" }
                        }
                    },
                    "constraints": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 4 }
                    },
                    "performance_requirements": {
                        "type": "array",
                        "items": { "type": "string", "minLength": 4 }
                    }
                }
            }),
            WizardPhase::GapAnalysis => serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "gaps",
                    "contradictions",
                    "recommendations",
                    "section_ratings",
                    "rewritten_sections"
                ],
                "properties": {
                    "gaps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["area", "issue", "suggestion"],
                            "properties": {
                                "area": { "type": "string" },
                                "issue": { "type": "string" },
                                "suggestion": { "type": "string" }
                            }
                        }
                    },
                    "contradictions": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["between", "issue"],
                            "properties": {
                                "between": { "type": "array", "items": { "type": "string" } },
                                "issue": { "type": "string" }
                            }
                        }
                    },
                    "recommendations": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["topic", "recommendation"],
                            "properties": {
                                "topic": { "type": "string" },
                                "recommendation": { "type": "string" }
                            }
                        }
                    },
                    "section_ratings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["section", "rating", "issues"],
                            "properties": {
                                "section": { "type": "string" },
                                "rating": { "type": "string", "enum": ["SPECIFIC", "NEEDS_DETAIL", "VAGUE"] },
                                "issues": { "type": "array", "items": { "type": "string" } }
                            }
                        }
                    },
                    "rewritten_sections": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["section", "rewritten"],
                            "properties": {
                                "section": { "type": "string" },
                                "rewritten": { "type": "string" }
                            }
                        }
                    }
                }
            }),
            WizardPhase::Generate => serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["sections", "terminology"],
                "properties": {
                    "sections": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["title", "content"],
                            "properties": {
                                "title": { "type": "string" },
                                "content": { "type": "string" }
                            }
                        }
                    },
                    "terminology": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["term", "definition", "category"],
                            "properties": {
                                "term": { "type": "string" },
                                "definition": { "type": "string" },
                                "category": { "type": "string" }
                            }
                        }
                    }
                }
            }),
            WizardPhase::TaskReview => serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["tasks", "removed", "added", "splits", "rewrites", "merges", "sizing_summary"],
                "properties": {
                    "tasks": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["description", "priority", "spec_section", "depends_on", "acceptance_criteria", "requires_browser_verification", "task_kind", "feature_group", "coverage_mode", "rationale", "size"],
                            "properties": {
                                "description": { "type": "string", "minLength": 6 },
                                "priority": { "type": "integer" },
                                "spec_section": { "type": ["string", "null"] },
                                "depends_on": { "type": "array", "items": { "type": "integer" } },
                                "acceptance_criteria": {
                                    "type": "array",
                                    "minItems": 1,
                                    "items": { "type": "string", "minLength": 4 }
                                },
                                "requires_browser_verification": { "type": "boolean" },
                                "task_kind": {
                                    "type": "string",
                                    "enum": ["feature", "test", "config", "docs", "verification", "refactor"]
                                },
                                "feature_group": { "type": "string", "minLength": 3 },
                                "coverage_mode": {
                                    "type": "string",
                                    "enum": ["inline", "dedicated", "none"]
                                },
                                "rationale": { "type": "string", "minLength": 6 },
                                "size": { "type": "string", "enum": ["S", "M", "L", "XL"] }
                            }
                        }
                    },
                    "removed": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["original", "reason"],
                            "properties": {
                                "original": { "type": "string" },
                                "reason": { "type": "string" }
                            }
                        }
                    },
                    "added": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["description", "reason"],
                            "properties": {
                                "description": { "type": "string" },
                                "reason": { "type": "string" }
                            }
                        }
                    },
                    "splits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["original", "into", "reason"],
                            "properties": {
                                "original": { "type": "string" },
                                "into": { "type": "array", "items": { "type": "string" } },
                                "reason": { "type": "string" }
                            }
                        }
                    },
                    "rewrites": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["original", "rewritten", "reason"],
                            "properties": {
                                "original": { "type": "string" },
                                "rewritten": { "type": "string" },
                                "reason": { "type": "string" }
                            }
                        }
                    },
                    "merges": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["merged", "into", "reason"],
                            "properties": {
                                "merged": { "type": "array", "items": { "type": "string" } },
                                "into": { "type": "string" },
                                "reason": { "type": "string" }
                            }
                        }
                    },
                    "sizing_summary": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["S", "M", "L", "XL", "total_splits", "total_rewrites", "total_merges"],
                        "properties": {
                            "S": { "type": "integer" },
                            "M": { "type": "integer" },
                            "L": { "type": "integer" },
                            "XL": { "type": "integer" },
                            "total_splits": { "type": "integer" },
                            "total_rewrites": { "type": "integer" },
                            "total_merges": { "type": "integer" }
                        }
                    }
                }
            }),
            WizardPhase::BuildTestConfig => serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "build_cmd",
                    "test_cmd",
                    "test_framework",
                    "pipeline_steps",
                    "test_tasks",
                    "build_timeout",
                    "test_timeout",
                    "rationale"
                ],
                "properties": {
                    "build_cmd": { "type": "string", "minLength": 1 },
                    "test_cmd": { "type": "string", "minLength": 1 },
                    "test_framework": { "type": "string", "minLength": 2 },
                    "pipeline_steps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["name", "command", "sort_order", "required", "timeout"],
                            "properties": {
                                "name": { "type": "string", "minLength": 2 },
                                "command": { "type": "string", "minLength": 1 },
                                "sort_order": { "type": "integer" },
                                "required": { "type": "boolean" },
                                "timeout": { "type": "integer" }
                            }
                        }
                    },
                    "test_tasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["description", "covers_features", "target_files", "rationale"],
                            "properties": {
                                "description": { "type": "string", "minLength": 8 },
                                "covers_features": {
                                    "type": "array",
                                    "minItems": 1,
                                    "items": { "type": "integer" }
                                },
                                "target_files": {
                                    "type": "array",
                                    "items": { "type": "string", "minLength": 3 }
                                },
                                "rationale": { "type": "string", "minLength": 6 }
                            }
                        }
                    },
                    "build_timeout": { "type": "integer" },
                    "test_timeout": { "type": "integer" },
                    "rationale": { "type": "string", "minLength": 8 }
                }
            }),
            WizardPhase::IterationMode => serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "recommended_mode",
                    "review_interval",
                    "ai_cli",
                    "subagent_timeout",
                    "rationale"
                ],
                "properties": {
                    "recommended_mode": {
                        "type": "string",
                        "enum": ["autonomous", "review_every", "review_each"]
                    },
                    "review_interval": { "type": ["integer", "null"] },
                    "ai_cli": {
                        "type": "string",
                        "enum": ["claude", "codex", "copilot", "gemini"]
                    },
                    "subagent_timeout": { "type": "integer" },
                    "rationale": { "type": "string" }
                }
            }),
            WizardPhase::Launch => return None,
        }
        .to_string(),
    )
}

/// Parse a JSON response from the provider, with one retry on failure.
/// Aggressive JSON extraction: find the outermost `{...}` or `[...]` in a string.
/// Used as a last-resort fallback when `extract_json` (markdown-aware) fails to
/// produce valid JSON.
fn extract_json_brute(text: &str) -> Option<String> {
    let trimmed = text.trim();
    // Find whichever comes first: `{` or `[`, and match to its closing counterpart
    let brace_pos = trimmed.find('{');
    let bracket_pos = trimmed.find('[');
    let (open, close) = match (brace_pos, bracket_pos) {
        (Some(b), Some(k)) => {
            if b < k {
                (b, '}')
            } else {
                (k, ']')
            }
        }
        (Some(b), None) => (b, '}'),
        (None, Some(k)) => (k, ']'),
        (None, None) => return None,
    };

    let bytes = trimmed.as_bytes();
    let open_char = bytes[open];
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;

    for i in open..bytes.len() {
        let b = bytes[i];
        if escape {
            escape = false;
            continue;
        }
        if b == b'\\' && in_string {
            escape = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if b == open_char {
            depth += 1;
        } else if b == close as u8 {
            depth -= 1;
            if depth == 0 {
                return Some(trimmed[open..=i].to_string());
            }
        }
    }
    None
}

async fn parse_json_with_repairs(
    response: &str,
    provider: &dyn Provider,
    phase: WizardPhase,
    original_prompt: &str,
    event_sink: Option<&WizardEventSink>,
) -> Result<JsonValue> {
    if let Some(value) = parse_json_candidate(response) {
        return Ok(value);
    }

    emit_wizard_event(
        event_sink,
        Event::Warning(format!(
            "Wizard phase {} returned invalid JSON. Attempting a JSON repair pass.",
            phase as i32
        )),
    );

    let repair_prompt = format!(
        r#"You are repairing malformed JSON for DIAL wizard phase {} ({}).

Convert the response below into valid JSON only.
- Preserve the original meaning and structure as much as possible
- Use double quotes for all keys and string values
- Remove markdown fences, comments, and trailing commas
- Return only a single valid JSON object or array, with no explanation

Malformed response:
```text
{}
```"#,
        phase as i32,
        phase.name(),
        truncate_for_prompt(response, 24000)
    );
    let repair_response =
        execute_wizard_prompt(provider, phase, &repair_prompt, event_sink).await?;
    if let Some(value) = parse_json_candidate(&repair_response) {
        return Ok(value);
    }

    emit_wizard_event(
        event_sink,
        Event::Warning(format!(
            "Wizard phase {} JSON repair failed. Regenerating with stricter instructions.",
            phase as i32
        )),
    );

    let retry_prompt = format!(
        "{}\n\nYour previous response was not valid JSON. Please respond with ONLY a valid JSON object. No markdown, no explanation, just JSON.",
        original_prompt
    );
    let retry_response = execute_wizard_prompt(provider, phase, &retry_prompt, event_sink).await?;
    if let Some(value) = parse_json_candidate(&retry_response) {
        return Ok(value);
    }

    emit_saved_debug_artifact(event_sink, phase, "original", response);
    emit_saved_debug_artifact(event_sink, phase, "repair", &repair_response);
    emit_saved_debug_artifact(event_sink, phase, "retry", &retry_response);

    Err(DialError::WizardError(
        "Failed to parse JSON response after multiple attempts. The AI provider returned invalid JSON.".to_string()
    ))
}

async fn enforce_phase_quality(
    value: JsonValue,
    provider: &dyn Provider,
    phase: WizardPhase,
    original_prompt: &str,
    event_sink: Option<&WizardEventSink>,
) -> Result<JsonValue> {
    if !should_enforce_phase_quality(provider.name()) {
        return Ok(value);
    }

    let issues = collect_phase_quality_issues(phase, &value);
    if issues.is_empty() {
        return Ok(value);
    }

    emit_wizard_event(
        event_sink,
        Event::Warning(format!(
            "Wizard phase {} returned generic JSON. Retrying with semantic quality guidance.",
            phase as i32
        )),
    );

    let retry_prompt = format!(
        r#"{original_prompt}

Your previous JSON parsed successfully, but it still failed quality checks for this phase:
{issues}

Correct the JSON now.
- Keep the same overall structure and required fields
- Replace generic names and placeholders with concrete domain-specific terms
- Do not use phrases like `unknown`, `feature name`, `workflow name`, `second entity`, `<entity>`, or `as defined in task 2`
- Return ONLY valid JSON, with no markdown or explanation"#,
        issues = issues
            .iter()
            .map(|issue| format!("- {}", issue))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let retry_response = execute_wizard_prompt(provider, phase, &retry_prompt, event_sink).await?;
    let corrected =
        parse_json_with_repairs(&retry_response, provider, phase, &retry_prompt, event_sink)
            .await?;
    let remaining = collect_phase_quality_issues(phase, &corrected);
    if remaining.is_empty() {
        return Ok(corrected);
    }

    emit_saved_debug_artifact(event_sink, phase, "quality", &retry_response);

    Err(DialError::WizardError(format!(
        "Wizard phase {} still returned generic placeholder content after a quality retry: {}",
        phase as i32,
        remaining.join(" | ")
    )))
}

async fn parse_json_response(
    response: &str,
    provider: &dyn Provider,
    phase: WizardPhase,
    original_prompt: &str,
    event_sink: Option<&WizardEventSink>,
) -> Result<JsonValue> {
    let parsed =
        parse_json_with_repairs(response, provider, phase, original_prompt, event_sink).await?;
    enforce_phase_quality(parsed, provider, phase, original_prompt, event_sink).await
}

fn parse_json_candidate(response: &str) -> Option<JsonValue> {
    let json_str = extract_json(response);
    if let Ok(value) = serde_json::from_str::<JsonValue>(&json_str) {
        return Some(value);
    }
    let normalized = normalize_wrapped_json(&json_str);
    if normalized != json_str {
        if let Ok(value) = serde_json::from_str::<JsonValue>(&normalized) {
            return Some(value);
        }
    }

    if let Some(brute) = extract_json_brute(response) {
        if let Ok(value) = serde_json::from_str::<JsonValue>(&brute) {
            return Some(value);
        }
        let normalized = normalize_wrapped_json(&brute);
        if normalized != brute {
            if let Ok(value) = serde_json::from_str::<JsonValue>(&normalized) {
                return Some(value);
            }
        }
    }

    None
}

fn normalize_wrapped_json(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut last_string_was_space = false;

    for ch in text.chars() {
        if in_string {
            if escaped {
                output.push(ch);
                escaped = false;
                last_string_was_space = false;
                continue;
            }

            match ch {
                '\\' => {
                    output.push(ch);
                    escaped = true;
                    last_string_was_space = false;
                }
                '"' => {
                    output.push(ch);
                    in_string = false;
                    last_string_was_space = false;
                }
                c if c.is_whitespace() => {
                    if !last_string_was_space {
                        output.push(' ');
                        last_string_was_space = true;
                    }
                }
                _ => {
                    output.push(ch);
                    last_string_was_space = false;
                }
            }
        } else {
            if ch == '"' {
                in_string = true;
            }
            output.push(ch);
        }
    }

    output
}

fn emit_saved_debug_artifact(
    event_sink: Option<&WizardEventSink>,
    phase: WizardPhase,
    kind: &str,
    response: &str,
) {
    if let Some(path) = save_debug_response(phase, kind, response) {
        emit_wizard_event(
            event_sink,
            Event::Warning(format!(
                "Saved wizard phase {} {} response for debugging: {}",
                phase as i32, kind, path
            )),
        );
    }
}

fn save_debug_response(phase: WizardPhase, kind: &str, response: &str) -> Option<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let filename = format!(
        "dial-wizard-phase-{}-{}-{}.txt",
        phase as i32, kind, timestamp
    );
    let path = std::env::temp_dir().join(filename);
    fs::write(&path, response).ok()?;
    Some(path.to_string_lossy().to_string())
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
    if maybe_store_source_requirements(state, from_doc) {
        save_wizard_state(prd_conn, state)?;
    }

    let mut sections_generated = 0;
    let mut tasks_generated = 0;

    // Phase 4: Gap Analysis with specificity check
    if !state
        .completed_phases
        .contains(&(WizardPhase::GapAnalysis as i32))
    {
        state.current_phase = WizardPhase::GapAnalysis;
        save_wizard_state(prd_conn, state)?;

        let prompt = build_phase_prompt(WizardPhase::GapAnalysis, state, from_doc);
        let response =
            execute_wizard_prompt(provider, WizardPhase::GapAnalysis, &prompt, None).await?;
        let data =
            parse_json_response(&response, provider, WizardPhase::GapAnalysis, &prompt, None)
                .await?;

        // Apply specificity rewrites to prd.db if sections exist
        let (_, rewrites) = parse_specificity_response(&data);
        if !rewrites.is_empty() {
            let _ = apply_specificity_rewrites(prd_conn, &rewrites);
        }

        state.set_phase_data(WizardPhase::GapAnalysis, data);
        state.mark_phase_complete(WizardPhase::GapAnalysis);
        save_wizard_state(prd_conn, state)?;
    }

    // Phase 5: Generate
    if !state
        .completed_phases
        .contains(&(WizardPhase::Generate as i32))
    {
        state.current_phase = WizardPhase::Generate;
        save_wizard_state(prd_conn, state)?;

        let prompt = build_phase_prompt(WizardPhase::Generate, state, from_doc);
        let response =
            execute_wizard_prompt(provider, WizardPhase::Generate, &prompt, None).await?;
        let data =
            parse_json_response(&response, provider, WizardPhase::Generate, &prompt, None).await?;

        // Insert generated sections into prd.db
        if let Some(sections) = data.get("sections").and_then(|s| s.as_array()) {
            crate::prd::prd_delete_all_sections(prd_conn)?;

            for (i, section) in sections.iter().enumerate() {
                let title = section
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Untitled");
                let content = section
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
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
                let canonical = term
                    .get("term")
                    .and_then(|t| t.as_str())
                    .unwrap_or_default();
                let definition = term
                    .get("definition")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default();
                let category = term
                    .get("category")
                    .and_then(|c| c.as_str())
                    .unwrap_or("general");

                if !canonical.is_empty() {
                    let _ = crate::prd::prd_add_term(
                        prd_conn, canonical, "[]", definition, category, None,
                    );
                }
            }
        }

        // Generate DIAL tasks from sections
        let phase_conn = crate::db::get_db(None)?;
        if let Some(sections) = data.get("sections").and_then(|s| s.as_array()) {
            for (i, section) in sections.iter().enumerate() {
                let title = section
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("Untitled");
                let desc = format!("Implement: {}", title);
                let priority = (i + 1) as i32;
                let section_id = format!("{}", i + 1);

                phase_conn.execute(
                    "INSERT INTO tasks (description, status, priority, prd_section_id)
                             VALUES (?1, 'pending', ?2, ?3)",
                    rusqlite::params![desc, priority, section_id],
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

/// Run the wizard as a single phase loop.
///
/// When `full` is true (used by `dial new`), runs all 9 phases.
/// When `full` is false (used by `dial spec wizard`), runs phases 1-5 only
/// for backward compatibility.
pub async fn run_wizard(
    provider: &dyn Provider,
    prd_conn: &Connection,
    template: &str,
    from_doc: Option<&str>,
    resume: bool,
    full: bool,
) -> Result<WizardResult> {
    run_wizard_with_events(provider, prd_conn, template, from_doc, resume, full, None).await
}

pub async fn run_wizard_with_events(
    provider: &dyn Provider,
    prd_conn: &Connection,
    template: &str,
    from_doc: Option<&str>,
    resume: bool,
    full: bool,
    event_sink: Option<WizardEventSink>,
) -> Result<WizardResult> {
    let mut state = if resume {
        load_wizard_state(prd_conn)?.unwrap_or_else(|| WizardState::new(template))
    } else {
        clear_wizard_state(prd_conn)?;
        WizardState::new(template)
    };

    // Validate template exists
    if get_template(&state.template).is_none() {
        return Err(DialError::TemplateNotFound(state.template.clone()));
    }

    if maybe_store_source_requirements(&mut state, from_doc) {
        save_wizard_state(prd_conn, &mut state)?;
    }

    let max_phase: i32 = if full { 9 } else { 5 };
    let mut result = WizardResult::default();

    if resume {
        let resumed_phase = if state.id > 0 {
            state.current_phase as u8
        } else {
            0
        };
        emit_wizard_event(
            event_sink.as_ref(),
            Event::WizardResumed {
                phase: resumed_phase,
            },
        );
    }

    for phase_num in 1..=max_phase {
        let phase = WizardPhase::from_i32(phase_num).unwrap();

        if state.completed_phases.contains(&phase_num) {
            continue;
        }

        emit_wizard_event(
            event_sink.as_ref(),
            Event::WizardPhaseStarted {
                phase: phase_num as u8,
                total_phases: max_phase as u8,
                name: phase.name().to_string(),
            },
        );
        let phase_started = Instant::now();

        let phase_result = match phase {
            // Phases 1-3: generic prompt → parse → store
            WizardPhase::Vision | WizardPhase::Functionality | WizardPhase::Technical => {
                state.current_phase = phase;
                save_wizard_state(prd_conn, &mut state)?;

                let prompt = build_phase_prompt(phase, &state, from_doc);
                let response =
                    execute_wizard_prompt(provider, phase, &prompt, event_sink.as_ref()).await?;
                let data =
                    parse_json_response(&response, provider, phase, &prompt, event_sink.as_ref())
                        .await?;
                state.set_phase_data(phase, data);
                state.mark_phase_complete(phase);
                save_wizard_state(prd_conn, &mut state)?;
                Ok(())
            }

            // Phase 4: gap analysis with specificity check
            WizardPhase::GapAnalysis => {
                state.current_phase = phase;
                save_wizard_state(prd_conn, &mut state)?;

                let prompt = build_phase_prompt(phase, &state, from_doc);
                let response =
                    execute_wizard_prompt(provider, phase, &prompt, event_sink.as_ref()).await?;
                let data =
                    parse_json_response(&response, provider, phase, &prompt, event_sink.as_ref())
                        .await?;

                // Apply specificity rewrites to prd.db if sections exist
                let (_, rewrites) = parse_specificity_response(&data);
                if !rewrites.is_empty() {
                    let _ = apply_specificity_rewrites(prd_conn, &rewrites);
                }

                state.set_phase_data(phase, data);
                state.mark_phase_complete(phase);
                save_wizard_state(prd_conn, &mut state)?;
                Ok(())
            }

            // Phase 5: generate PRD sections, terminology, and DIAL tasks
            WizardPhase::Generate => {
                state.current_phase = phase;
                save_wizard_state(prd_conn, &mut state)?;

                let prompt = build_phase_prompt(phase, &state, from_doc);
                let response =
                    execute_wizard_prompt(provider, phase, &prompt, event_sink.as_ref()).await?;
                let data =
                    parse_json_response(&response, provider, phase, &prompt, event_sink.as_ref())
                        .await?;

                // Insert generated sections into prd.db
                if let Some(sections) = data.get("sections").and_then(|s| s.as_array()) {
                    crate::prd::prd_delete_all_sections(prd_conn)?;

                    for (i, section) in sections.iter().enumerate() {
                        let title = section
                            .get("title")
                            .and_then(|t| t.as_str())
                            .unwrap_or("Untitled");
                        let content = section
                            .get("content")
                            .and_then(|c| c.as_str())
                            .unwrap_or("");
                        let word_count = content.split_whitespace().count() as i32;
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
                        result.sections_generated += 1;
                    }
                }

                // Extract and store terminology
                if let Some(terms) = data.get("terminology").and_then(|t| t.as_array()) {
                    for term in terms {
                        let canonical = term
                            .get("term")
                            .and_then(|t| t.as_str())
                            .unwrap_or_default();
                        let definition = term
                            .get("definition")
                            .and_then(|d| d.as_str())
                            .unwrap_or_default();
                        let category = term
                            .get("category")
                            .and_then(|c| c.as_str())
                            .unwrap_or("general");

                        if !canonical.is_empty() {
                            let _ = crate::prd::prd_add_term(
                                prd_conn, canonical, "[]", definition, category, None,
                            );
                        }
                    }
                }

                // Generate DIAL tasks from sections
                let phase_conn = crate::db::get_db(None)?;
                if let Some(sections) = data.get("sections").and_then(|s| s.as_array()) {
                    for (i, section) in sections.iter().enumerate() {
                        let title = section
                            .get("title")
                            .and_then(|t| t.as_str())
                            .unwrap_or("Untitled");
                        let desc = format!("Implement: {}", title);
                        let priority = (i + 1) as i32;
                        let section_id = format!("{}", i + 1);

                        phase_conn.execute(
                            "INSERT INTO tasks (description, status, priority, prd_section_id)
                             VALUES (?1, 'pending', ?2, ?3)",
                            rusqlite::params![desc, priority, section_id],
                        )?;
                        result.tasks_generated += 1;
                    }
                }

                state.set_phase_data(phase, data);
                state.mark_phase_complete(phase);
                save_wizard_state(prd_conn, &mut state)?;
                Ok(())
            }

            // Phase 6: task review with sizing analysis
            WizardPhase::TaskReview => {
                let (kept, added, removed, sizing) =
                    run_wizard_phase_6(provider, prd_conn, &mut state, event_sink.as_ref()).await?;
                result.tasks_kept = kept;
                result.tasks_added = added;
                result.tasks_removed = removed;
                result.sizing_summary = sizing;
                Ok(())
            }

            // Phase 7: build & test config with test strategy
            WizardPhase::BuildTestConfig => {
                let (build_cmd, test_cmd, steps, test_tasks) =
                    run_wizard_phase_7(provider, prd_conn, &mut state, event_sink.as_ref()).await?;
                result.build_cmd = build_cmd;
                result.test_cmd = test_cmd;
                result.pipeline_steps = steps;
                result.test_tasks_added = test_tasks;
                Ok(())
            }

            // Phase 8: iteration mode
            WizardPhase::IterationMode => {
                let mode =
                    run_wizard_phase_8(provider, prd_conn, &mut state, event_sink.as_ref()).await?;
                result.iteration_mode = mode;
                Ok(())
            }

            // Phase 9: launch summary (no provider call)
            WizardPhase::Launch => {
                let summary = run_wizard_phase_9(prd_conn, &mut state)?;
                result.project_name = summary.project_name;
                result.task_count = summary.task_count;
                result.build_cmd = summary.build_cmd;
                result.test_cmd = summary.test_cmd;
                result.iteration_mode = summary.iteration_mode;
                result.ai_cli = summary.ai_cli;
                Ok(())
            }
        };

        if let Err(error) = phase_result {
            emit_wizard_event(
                event_sink.as_ref(),
                Event::Warning(format!(
                    "Wizard stopped at phase {}. State was saved and can be resumed.",
                    phase_num
                )),
            );
            emit_wizard_event(
                event_sink.as_ref(),
                Event::WizardPaused {
                    phase: phase_num as u8,
                },
            );
            return Err(error);
        }

        let phase_duration = phase_started.elapsed().as_secs_f64();
        emit_wizard_event(
            event_sink.as_ref(),
            Event::WizardPhaseCompleted {
                phase: phase_num as u8,
                name: phase.name().to_string(),
            },
        );
        if full && matches!(phase, WizardPhase::Generate) {
            emit_wizard_event(
                event_sink.as_ref(),
                Event::WizardCheckpoint {
                    phase: phase_num as u8,
                    title: "Planning checkpoint".to_string(),
                    message: "The PRD and initial task list are ready. DIAL is still in planning/configuration mode and has not started implementation.".to_string(),
                    next_step: Some(
                        "Next: task review, build/test configuration, and iteration mode setup."
                            .to_string(),
                    ),
                },
            );
        }
        emit_wizard_event(
            event_sink.as_ref(),
            Event::Info(format!(
                "Wizard phase {} finished in {:.1}s",
                phase_num, phase_duration
            )),
        );
    }

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

    let source_requirements = build_source_requirements_context(gathered_info);
    let project_summary = build_project_summary_context(gathered_info);
    let section_outline = build_generated_section_outline(gathered_info);

    format!(
        r#"You are a senior software architect reviewing and refining a task list generated from a PRD.

## Current Task List (generated from PRD)
{task_list}
{source_requirements}{project_summary}{section_outline}

## SOURCE FIDELITY

If source requirements are present above, they are authoritative:
- Preserve exact literal values from them, including enum or status names, commands, file paths, file names, and acceptance checks.
- Do not introduce alternate contracts, validation rules, or extra scope that the source requirements do not ask for.
- If the source requirements prefer a task-count range, keep the final task list within that range unless it is impossible to satisfy the scope.
- Do not create duplicate tasks for behavior or test coverage that is already fully owned by another task.

## TASK SIZING ANALYSIS

Before producing the final task list, evaluate EVERY task on three dimensions:

1. **SCOPE**: Each task should touch 1-3 files and do ONE thing. If a task requires changes to more than 3 files or implements multiple distinct features, it must be SPLIT.

2. **SPECIFICITY**: Each task description must be concrete enough for an AI agent to implement without guessing.
   - BAD: "Build auth system" (vague, multi-step)
   - BAD: "Set up database" (unclear what tables/schema)
   - GOOD: "Add bcrypt password hashing to User model with cost factor 12"
   - GOOD: "Create users table with columns: id, email, password_hash, created_at"

3. **TESTABILITY**: Success must be verifiable by running build + tests. If a task cannot be validated by automated checks, rewrite it so it can be.
4. **NO PLACEHOLDERS**: Every task must stand on its own using concrete project nouns. Do not use phrases like `second entity`, `<entity>`, `feature name`, or `as defined in task 2`.
5. **SOURCE FIDELITY**: Task descriptions must faithfully implement the requested behavior. Do not swap in different enum values, file names, CLI behaviors, or validation rules.
6. **ACCEPTANCE CRITERIA**: Every task must include 1-3 short acceptance criteria describing the observable result of the work.
7. **BROWSER VERIFICATION**: Set `requires_browser_verification` to `true` only when the task changes user-facing UI behavior that should be checked manually in a browser. Leave it `false` for backend, CLI, data, or fully automated work.
8. **TASK KIND**: Classify every task as one of `feature`, `test`, `config`, `docs`, `verification`, or `refactor`.
9. **FEATURE GROUP**: Assign every task a stable kebab-case `feature_group` that identifies the product slice it belongs to. Tasks that serve the same feature slice should share the same group.
10. **COVERAGE OWNERSHIP**: Set `coverage_mode` to `inline` when the task itself should carry its automated coverage, `dedicated` when this task is the single dedicated coverage owner for its feature_group, or `none` when coverage ownership does not apply.
11. **NO TRAILING VERIFICATION TASKS**: Do not leave a standalone `verification` task for generic acceptance checks when the same build, test, or browser-visible checks can be attached to the relevant feature and dedicated test tasks. Reserve `verification` tasks for rare work that truly cannot be absorbed elsewhere.

### Actions Required

- **SPLIT** any task that requires more than 3 files OR implements multiple features. Create sub-tasks with explicit dependency relationships between them (the sub-tasks should appear in order in the tasks array, with later ones depending on earlier ones).
- **REWRITE** any vague task description to be concrete with specific inputs, outputs, and acceptance criteria.
- **MERGE** tasks that are too small for a separate iteration (e.g., single-line config changes) into a related neighboring task.
- **DEDUPE** tasks that repeat work already completed by another task or that describe the same acceptance check twice.
- **SIZE** every task as one of: [S]mall (1 file, <15 min), [M]edium (1-2 files, ~30 min), [L]arge (2-3 files, ~45 min), [XL]needs-review (>3 files or >1 hour; should be split further).

Any task sized [XL] MUST be split. Do not leave XL tasks in the final list.

## Review Steps

1. Reorder by logical implementation sequence (foundation first, then features, then polish)
2. Add any missing tasks needed for a complete implementation
3. Remove redundant or overly-granular tasks
4. Set dependency relationships using 0-based indices into your output tasks array
5. Assign realistic priorities (1 = implement first, higher numbers = implement later)
6. Include 1-3 concrete acceptance criteria for every task
7. Mark browser verification only for tasks that truly need a manual UI check
8. Keep coverage ownership unambiguous: at most one `dedicated` test task per `feature_group`
9. Fold generic acceptance-check-only work into related feature or test tasks instead of creating a trailing verification task

Each task should be roughly one commit's worth of work (~30 minutes).
In the `depends_on` array, use 0-based indices referring to other tasks in YOUR output array.
For example, if the task at index 2 depends on the task at index 0, set `"depends_on": [0]`.
Every task description must be self-contained. Do not refer to "the previous task", "task 2", or unnamed entities.

Respond in JSON format:
{{
  "tasks": [
    {{"description": "concrete task description", "priority": 1, "spec_section": "1.2", "depends_on": [], "acceptance_criteria": ["observable result one", "observable result two"], "requires_browser_verification": false, "task_kind": "feature", "feature_group": "auth-login", "coverage_mode": "inline", "rationale": "why this order", "size": "S"}}
  ],
  "removed": [
    {{"original": "task that was removed", "reason": "why"}}
  ],
  "added": [
    {{"description": "new task", "reason": "why it was missing"}}
  ],
  "splits": [
    {{"original": "original task that was too large", "into": ["sub-task 1 description", "sub-task 2 description"], "reason": "why it needed splitting"}}
  ],
  "rewrites": [
    {{"original": "vague task description", "rewritten": "concrete task description", "reason": "what was vague"}}
  ],
  "merges": [
    {{"merged": ["small task 1", "small task 2"], "into": "combined task description", "reason": "why they were merged"}}
  ],
  "sizing_summary": {{
    "S": 0, "M": 0, "L": 0, "XL": 0,
    "total_splits": 0, "total_rewrites": 0, "total_merges": 0
  }}
}}

Respond ONLY with valid JSON."#
    )
}

/// Read all pending/in-progress tasks from the DIAL database as tuples.
fn read_task_list(conn: &Connection) -> Result<Vec<(i64, String, i32, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT id, description, priority,
                COALESCE(prd_section_id, CAST(spec_section_id AS TEXT))
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

fn task_explicitly_owns_test_coverage(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    [
        "add tests",
        "add test",
        "write tests",
        "write test",
        "test coverage",
        "coverage in ",
        "integration test",
        "integration coverage",
        "unit test",
        "cli coverage",
        "test/",
        "tests/",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn task_is_manual_browser_verification(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    (lower.contains("manually verify") || lower.contains("manual browser verification"))
        && lower.contains("browser")
}

fn pipeline_step_is_brittle_inline_eval(command: &str) -> bool {
    let normalized = command.to_ascii_lowercase();
    let has_inline_node_eval =
        normalized.contains("node -e ") || normalized.contains("node --eval ");
    has_inline_node_eval && (command.len() > 220 || command.contains('\''))
}

fn normalize_task_text_for_dedupe(text: &str) -> String {
    let normalized: String = text
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_feature_group(value: &str) -> Option<String> {
    let normalized = normalize_task_text_for_dedupe(value).replace(' ', "-");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn normalize_task_kind(value: &str) -> String {
    match normalize_task_text_for_dedupe(value).as_str() {
        "feature" | "test" | "config" | "docs" | "verification" | "refactor" => {
            normalize_task_text_for_dedupe(value)
        }
        _ => "feature".to_string(),
    }
}

fn normalize_coverage_mode(value: &str) -> String {
    match normalize_task_text_for_dedupe(value).as_str() {
        "inline" | "dedicated" | "none" => normalize_task_text_for_dedupe(value),
        _ => "none".to_string(),
    }
}

fn parse_phase6_task_plan_records(
    gathered_info: &JsonValue,
) -> std::collections::HashMap<String, Phase6TaskPlanRecord> {
    gathered_info
        .get("task_review")
        .and_then(|phase| phase.get("tasks"))
        .and_then(|tasks| tasks.as_array())
        .map(|tasks| {
            tasks
                .iter()
                .filter_map(|task| {
                    let description = task.get("description")?.as_str()?.trim();
                    if description.is_empty() {
                        return None;
                    }

                    Some((
                        normalize_task_text_for_dedupe(description),
                        Phase6TaskPlanRecord {
                            task_kind: normalize_task_kind(
                                task.get("task_kind")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("feature"),
                            ),
                            feature_group: task
                                .get("feature_group")
                                .and_then(|value| value.as_str())
                                .and_then(normalize_feature_group),
                            coverage_mode: normalize_coverage_mode(
                                task.get("coverage_mode")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or("none"),
                            ),
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn task_path_hints(text: &str) -> std::collections::HashSet<String> {
    text.split_whitespace()
        .filter_map(|token| {
            let cleaned = token.trim_matches(|ch: char| {
                !(ch.is_ascii_alphanumeric() || ch == '/' || ch == '.' || ch == '_' || ch == '-')
            });
            let lower = cleaned.to_ascii_lowercase();
            if lower.is_empty() {
                return None;
            }
            if !(lower.contains('/') || lower.contains('.')) {
                return None;
            }
            if !lower.chars().any(|ch| ch.is_ascii_alphabetic()) {
                return None;
            }
            Some(lower)
        })
        .collect()
}

fn normalized_path_list(paths: &[String]) -> std::collections::HashSet<String> {
    paths
        .iter()
        .filter_map(|path| {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_ascii_lowercase())
            }
        })
        .collect()
}

fn descriptions_materially_overlap(lhs: &str, rhs: &str) -> bool {
    let lhs_keywords = task_scope_keywords(lhs);
    let rhs_keywords = task_scope_keywords(rhs);
    if lhs_keywords.is_empty() || rhs_keywords.is_empty() {
        return false;
    }
    let overlap = lhs_keywords.intersection(&rhs_keywords).count();
    let required_overlap = std::cmp::min(3, lhs_keywords.len().min(rhs_keywords.len()));
    overlap >= required_overlap
}

fn task_scope_keywords(text: &str) -> std::collections::HashSet<String> {
    const STOP_WORDS: &[&str] = &[
        "a",
        "add",
        "all",
        "and",
        "automated",
        "coverage",
        "execute",
        "existing",
        "extend",
        "for",
        "from",
        "implement",
        "in",
        "include",
        "includes",
        "into",
        "message",
        "note",
        "object",
        "of",
        "one",
        "only",
        "package",
        "preserve",
        "preserving",
        "print",
        "prints",
        "required",
        "return",
        "returns",
        "scripts",
        "so",
        "test",
        "tests",
        "that",
        "the",
        "to",
        "update",
        "using",
        "valid",
        "verifies",
        "verify",
        "when",
        "while",
        "with",
    ];

    normalize_task_text_for_dedupe(text)
        .split_whitespace()
        .filter(|token| token.len() >= 3)
        .filter(|token| !STOP_WORDS.contains(token))
        .map(|token| token.to_string())
        .collect()
}

fn task_covers_feature_scope(existing_task: &str, feature_description: &str) -> bool {
    if !task_explicitly_owns_test_coverage(existing_task) {
        return false;
    }

    let feature_keywords = task_scope_keywords(feature_description);
    if feature_keywords.is_empty() {
        return false;
    }

    let existing_keywords = task_scope_keywords(existing_task);
    let overlap = feature_keywords.intersection(&existing_keywords).count();
    let required_overlap = std::cmp::min(4, feature_keywords.len());

    overlap >= required_overlap
}

#[derive(Clone, Debug)]
struct ReviewedTask {
    description: String,
    priority: i32,
    prd_section_id: Option<String>,
    depends_on: Vec<usize>,
    acceptance_criteria: Vec<String>,
    requires_browser_verification: bool,
    task_kind: String,
    feature_group: Option<String>,
}

fn matching_feature_dependencies(tasks: &[ReviewedTask], test_index: usize) -> Vec<usize> {
    let test_description = &tasks[test_index].description;

    tasks
        .iter()
        .enumerate()
        .filter_map(|(index, task)| {
            if index == test_index || task_explicitly_owns_test_coverage(&task.description) {
                return None;
            }

            if task_covers_feature_scope(test_description, &task.description) {
                Some(index)
            } else {
                None
            }
        })
        .collect()
}

fn task_is_generic_acceptance_verification(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    lower.contains("run the acceptance checks")
        || lower.contains("acceptance checks")
        || lower.contains("acceptance criteria")
        || (lower.contains("browser-visible")
            && (lower.contains("npm run build") || lower.contains("npm test")))
}

fn choose_verification_anchor(tasks: &[ReviewedTask], verification_index: usize) -> Option<usize> {
    let verification_task = tasks.get(verification_index)?;
    let dependencies: Vec<usize> = verification_task
        .depends_on
        .iter()
        .copied()
        .filter(|index| *index < tasks.len() && *index != verification_index)
        .collect();

    let same_group = |index: usize| {
        verification_task.feature_group.is_some()
            && tasks[index].feature_group == verification_task.feature_group
    };

    let find_match = |predicate: &dyn Fn(usize) -> bool| -> Option<usize> {
        dependencies
            .iter()
            .copied()
            .find(|index| predicate(*index))
            .or_else(|| {
                tasks.iter().enumerate().find_map(|(index, _)| {
                    if index == verification_index || !predicate(index) {
                        None
                    } else {
                        Some(index)
                    }
                })
            })
    };

    find_match(&|index| {
        let task = &tasks[index];
        same_group(index) && task.task_kind != "verification" && task.task_kind != "test"
    })
    .or_else(|| {
        find_match(&|index| {
            let task = &tasks[index];
            same_group(index) && task.task_kind != "verification"
        })
    })
    .or_else(|| {
        find_match(&|index| {
            let task = &tasks[index];
            task.task_kind != "verification" && task.task_kind != "test"
        })
    })
    .or_else(|| find_match(&|index| tasks[index].task_kind != "verification"))
}

fn merge_acceptance_criteria(target: &mut Vec<String>, source: &[String]) {
    for criterion in source {
        if !target.iter().any(|existing| existing == criterion) {
            target.push(criterion.clone());
        }
    }
}

fn fold_standalone_verification_tasks(reviewed_tasks: &mut Vec<ReviewedTask>) {
    if reviewed_tasks.is_empty() {
        return;
    }

    let mut folded_into = vec![None; reviewed_tasks.len()];

    for index in 0..reviewed_tasks.len() {
        let Some(task) = reviewed_tasks.get(index).cloned() else {
            continue;
        };
        if task.task_kind != "verification"
            || !task_is_generic_acceptance_verification(&task.description)
        {
            continue;
        }

        let Some(anchor_index) = choose_verification_anchor(reviewed_tasks, index) else {
            continue;
        };

        if let Some(anchor_task) = reviewed_tasks.get_mut(anchor_index) {
            anchor_task.requires_browser_verification |= task.requires_browser_verification
                || task.description.to_ascii_lowercase().contains("browser");
            merge_acceptance_criteria(
                &mut anchor_task.acceptance_criteria,
                &task.acceptance_criteria,
            );
            folded_into[index] = Some(anchor_index);
        }
    }

    if !folded_into.iter().any(|item| item.is_some()) {
        return;
    }

    let mut old_to_new: Vec<Option<usize>> = vec![None; reviewed_tasks.len()];
    let mut retained_tasks = Vec::with_capacity(reviewed_tasks.len());

    for (index, task) in reviewed_tasks.iter().cloned().enumerate() {
        if folded_into[index].is_some() {
            continue;
        }
        old_to_new[index] = Some(retained_tasks.len());
        retained_tasks.push(task);
    }

    for (old_index, task) in reviewed_tasks.iter().enumerate() {
        if folded_into[old_index].is_some() {
            continue;
        }
        let new_index = old_to_new[old_index].expect("retained tasks must have a mapped index");
        let retained_task = &mut retained_tasks[new_index];
        let mut remapped_dependencies = Vec::with_capacity(task.depends_on.len());

        for dep_index in &task.depends_on {
            if *dep_index >= folded_into.len() {
                continue;
            }
            let mapped_old_index = folded_into[*dep_index].unwrap_or(*dep_index);
            if mapped_old_index == old_index {
                continue;
            }
            if let Some(mapped_new_index) = old_to_new[mapped_old_index] {
                if mapped_new_index != new_index {
                    remapped_dependencies.push(mapped_new_index);
                }
            }
        }

        remapped_dependencies.sort_unstable();
        remapped_dependencies.dedup();
        retained_task.depends_on = remapped_dependencies;
    }

    *reviewed_tasks = retained_tasks;
}

/// Run wizard phase 6: Task Review with Sizing Analysis.
///
/// Reads tasks generated by phase 5 from the DIAL database, sends them
/// to the AI provider for review, sizing analysis, and refinement, then
/// replaces the original tasks with the reviewed versions including
/// dependency relationships and sizing annotations.
///
/// Returns (tasks_kept, tasks_added, tasks_removed, sizing_summary).
pub async fn run_wizard_phase_6(
    provider: &dyn Provider,
    prd_conn: &Connection,
    state: &mut WizardState,
    event_sink: Option<&WizardEventSink>,
) -> Result<(usize, usize, usize, SizingSummary)> {
    if state
        .completed_phases
        .contains(&(WizardPhase::TaskReview as i32))
    {
        return Ok((0, 0, 0, SizingSummary::default()));
    }

    state.current_phase = WizardPhase::TaskReview;
    save_wizard_state(prd_conn, state)?;

    // 1. Read existing tasks from the DIAL database
    let phase_conn = crate::db::get_db(None)?;
    let tasks = read_task_list(&phase_conn)?;

    // 2. Build the prompt with task list, PRD context, and sizing instructions
    let prompt = build_task_review_prompt(&tasks, &state.gathered_info);

    // 3. Send to provider and parse JSON response
    let response =
        execute_wizard_prompt(provider, WizardPhase::TaskReview, &prompt, event_sink).await?;
    let data = parse_json_response(
        &response,
        provider,
        WizardPhase::TaskReview,
        &prompt,
        event_sink,
    )
    .await?;

    // 4. Parse sizing analysis data (splits, rewrites, merges, summary)
    let (_splits, _rewrites, _merges, sizing) = parse_sizing_response(&data);

    // 5. Replace tasks in the database with reviewed versions
    let (kept, added, removed) = apply_task_review(&phase_conn, &data)?;

    // 6. Store the review data in wizard state
    state.set_phase_data(WizardPhase::TaskReview, data);
    state.mark_phase_complete(WizardPhase::TaskReview);
    save_wizard_state(prd_conn, state)?;

    Ok((kept, added, removed, sizing))
}

/// Apply the AI-reviewed task list to the database.
///
/// Deletes all existing pending tasks (from phase 5), inserts the reviewed
/// task list, and sets up dependency relationships between them.
///
/// Returns (tasks_kept, tasks_added, tasks_removed) counts.
pub fn apply_task_review(
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

    let mut reviewed_tasks: Vec<ReviewedTask> = tasks
        .iter()
        .map(|task| ReviewedTask {
            description: task
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("Untitled task")
                .to_string(),
            priority: task.get("priority").and_then(|p| p.as_i64()).unwrap_or(5) as i32,
            prd_section_id: task
                .get("spec_section")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string()),
            depends_on: task
                .get("depends_on")
                .and_then(|d| d.as_array())
                .map(|deps| {
                    deps.iter()
                        .filter_map(|dep| dep.as_u64().map(|value| value as usize))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            acceptance_criteria: task
                .get("acceptance_criteria")
                .and_then(|criteria| criteria.as_array())
                .map(|criteria| {
                    criteria
                        .iter()
                        .filter_map(|criterion| criterion.as_str())
                        .map(|criterion| criterion.trim().to_string())
                        .filter(|criterion| !criterion.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            requires_browser_verification: task
                .get("requires_browser_verification")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            task_kind: normalize_task_kind(
                task.get("task_kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or("feature"),
            ),
            feature_group: task
                .get("feature_group")
                .and_then(|value| value.as_str())
                .and_then(normalize_feature_group),
        })
        .collect();

    for index in 0..reviewed_tasks.len() {
        if !task_explicitly_owns_test_coverage(&reviewed_tasks[index].description) {
            continue;
        }

        let matches = matching_feature_dependencies(&reviewed_tasks, index);
        if matches.is_empty() {
            continue;
        }

        let max_dependency_priority = matches
            .iter()
            .filter_map(|dep_index| reviewed_tasks.get(*dep_index))
            .map(|task| task.priority)
            .max()
            .unwrap_or(reviewed_tasks[index].priority);

        for dep_index in matches {
            if !reviewed_tasks[index].depends_on.contains(&dep_index) {
                reviewed_tasks[index].depends_on.push(dep_index);
            }
        }
        reviewed_tasks[index].depends_on.sort_unstable();
        reviewed_tasks[index].depends_on.dedup();
        reviewed_tasks[index].priority = reviewed_tasks[index]
            .priority
            .max(max_dependency_priority + 1);
    }

    fold_standalone_verification_tasks(&mut reviewed_tasks);

    // Delete existing pending/in-progress tasks and their dependencies (both directions)
    conn.execute(
        "DELETE FROM task_dependencies WHERE task_id IN (
            SELECT id FROM tasks WHERE status IN ('pending', 'in_progress')
        ) OR depends_on_id IN (
            SELECT id FROM tasks WHERE status IN ('pending', 'in_progress')
        )",
        [],
    )?;
    conn.execute(
        "DELETE FROM tasks WHERE status IN ('pending', 'in_progress')",
        [],
    )?;

    // Insert reviewed tasks and collect their new IDs (indexed by position)
    let mut new_task_ids: Vec<i64> = Vec::with_capacity(reviewed_tasks.len());

    for task in &reviewed_tasks {
        let acceptance_criteria_json = if task.acceptance_criteria.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&task.acceptance_criteria).map_err(|err| {
                    DialError::WizardError(format!(
                        "Failed to serialize task acceptance criteria: {}",
                        err
                    ))
                })?,
            )
        };
        conn.execute(
            "INSERT INTO tasks (
                description, status, priority, prd_section_id, acceptance_criteria_json, requires_browser_verification
             ) VALUES (?1, 'pending', ?2, ?3, ?4, ?5)",
            params![
                task.description,
                task.priority,
                task.prd_section_id,
                acceptance_criteria_json,
                if task.requires_browser_verification { 1 } else { 0 }
            ],
        )?;
        new_task_ids.push(conn.last_insert_rowid());
    }

    // Set up dependency relationships using 0-based indices
    for (i, task) in reviewed_tasks.iter().enumerate() {
        let task_id = new_task_ids[i];
        for dep_idx in &task.depends_on {
            if *dep_idx < new_task_ids.len() && *dep_idx != i {
                let depends_on_id = new_task_ids[*dep_idx];
                conn.execute(
                    "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id)
                     VALUES (?1, ?2)",
                    params![task_id, depends_on_id],
                )?;
            }
        }
    }

    let kept_count = reviewed_tasks.len().saturating_sub(added_count);

    Ok((kept_count, added_count, removed_count))
}

/// Build the phase 7 (Build & Test Configuration) prompt with technical details.
///
/// Unlike generic phases that only use `gathered_info` as prior context, phase 7
/// extracts the technical details from phase 3 and presents them prominently
/// so the AI can recommend appropriate build/test commands and pipeline steps.
/// It also includes the current feature task list so the AI can generate
/// dedicated test tasks with dependency relationships.
///
/// # Arguments
/// * `gathered_info` - Accumulated wizard state from phases 1-6
/// * `tasks` - Current feature tasks from the database (id, description, priority, section)
pub fn build_build_test_config_prompt(
    gathered_info: &JsonValue,
    tasks: &[(i64, String, i32, Option<String>)],
) -> String {
    let source_requirements = build_source_requirements_context(gathered_info);
    let technical_context = if let Some(technical) = gathered_info.get("technical") {
        format!(
            "\n## Technical Details (from Phase 3)\n```json\n{}\n```\n",
            serde_json::to_string_pretty(technical).unwrap_or_default()
        )
    } else {
        "\nNo technical details available from prior phases.\n".to_string()
    };

    let project_summary = build_project_summary_context(gathered_info);
    let section_outline = build_generated_section_outline(gathered_info);
    let phase6_task_plans = parse_phase6_task_plan_records(gathered_info);

    let task_list = if tasks.is_empty() {
        "No tasks available.".to_string()
    } else {
        let items: Vec<String> = tasks
            .iter()
            .enumerate()
            .map(|(idx, (_id, desc, priority, section))| {
                let section_str = section.as_deref().unwrap_or("none");
                let task_key = normalize_task_text_for_dedupe(desc);
                let metadata = phase6_task_plans.get(&task_key);
                let task_kind = metadata
                    .map(|plan| plan.task_kind.as_str())
                    .unwrap_or_else(|| {
                        if task_explicitly_owns_test_coverage(desc) {
                            "test"
                        } else {
                            "feature"
                        }
                    });
                let coverage_mode = metadata
                    .map(|plan| plan.coverage_mode.as_str())
                    .unwrap_or_else(|| {
                        if task_explicitly_owns_test_coverage(desc) {
                            "dedicated"
                        } else {
                            "inline"
                        }
                    });
                let feature_group = metadata
                    .and_then(|plan| plan.feature_group.as_deref())
                    .unwrap_or("unspecified");
                format!(
                    "  [{}] P{} (section: {}) {} [kind: {} | coverage: {} | group: {}]",
                    idx, priority, section_str, desc, task_kind, coverage_mode, feature_group
                )
            })
            .collect();
        items.join("\n")
    };

    format!(
        r#"You are configuring build and test commands for a software project.
{source_requirements}
{technical_context}
{project_summary}{section_outline}
Based on the technical details above (languages, frameworks, platform, constraints),
suggest the appropriate build and test commands and a validation pipeline.
Preserve exact commands, file paths, stdin/stdout behavior, and acceptance checks from the source requirements when they are explicitly provided.

The pipeline_steps should cover all validation concerns for this project
(e.g., linting, building, testing, integration tests). Order them by execution sequence.

## TEST STRATEGY

Review the current task list below and determine test coverage needs:

### Current Task List (0-indexed)
{task_list}

Phase 6 already assigned each task a `kind`, `coverage`, and `group`.
Treat those planning decisions as authoritative unless they are obviously inconsistent with the requirements.

For EACH feature-facing slice, decide:

1. **Complex features** (multi-file, API endpoints, data models, state management) get a DEDICATED test task that depends on the feature task. Write specific test descriptions with concrete scenarios:
   - BAD: "Write tests for user module"
   - GOOD: "Write integration tests for POST /users: valid input returns 201, duplicate email returns 409, missing required fields returns 422"

2. **Simple features** (config changes, single-function utilities, constants) include tests inline with the feature; no separate test task needed.

3. If Phase 6 already marked a task as `kind: test` or `coverage: dedicated` for a given `group`, do NOT emit another dedicated `test_task` for that same `group`.
4. If a feature task description already explicitly includes its required tests or coverage, do NOT emit a separate `test_task` for that feature. Keep the verification inline with that feature task.
5. Do NOT emit a separate `test_task` whose only purpose is manual browser verification for a UI feature. Keep manual browser verification attached to the feature task instead of turning it into a new standalone task.
6. At most one dedicated test task may own a given `group`.
7. When one dedicated test task intentionally covers multiple adjacent feature groups, keep that work in a single task and list all covered feature indices in `covers_features` instead of duplicating neighboring test tasks.

Use concrete project terminology from the earlier phases. Do not use placeholders like `module`, `entity`, `<route>`, or `as defined in task 2`.

### Test Framework

Based on the tech stack, suggest the appropriate test framework (e.g., `cargo test` for Rust, `pytest` for Python, `jest` for JavaScript/TypeScript, `go test` for Go).

### Validation Pipeline

Suggest pipeline steps with `sort_order` (execution sequence), `required` flag (must pass to continue), and `timeout` in seconds:
- Linting: typically optional, fast timeout
- Build: required, medium timeout
- Unit tests: required, medium timeout
- Integration tests: required if applicable, longer timeout

Important pipeline rules:
- Required pipeline steps must be safe to run after EVERY task, not just after the final project state. Do not add a required smoke test or end-to-end check that depends on later tasks or future files.
- If a validation check only makes sense after a later feature task is complete, represent it as a `test_task` or make the pipeline step optional.
- Prefer the existing `build_cmd` and `test_cmd` as the required gates. Additional pipeline steps should usually be optional preflight checks.
- Match the host shell for the project platform. For local Windows command-line projects, prefer simple `node`, `npm`, or Windows PowerShell 5.1 compatible commands. Do not emit POSIX-only quoting or bash-specific escape patterns.
- For local Windows Node.js projects, do not emit inline `node -e` or `node --eval` pipeline steps. Prefer `node --check`, `npm test`, `npm run build`, or a dedicated `test_task` instead.

Respond in JSON format:
{{
  "build_cmd": "the primary build command",
  "test_cmd": "the primary test command",
  "test_framework": "recommended test framework and runner command",
  "pipeline_steps": [
    {{"name": "step name", "command": "shell command", "sort_order": 1, "required": false, "timeout": 120}},
    {{"name": "step name", "command": "shell command", "sort_order": 2, "required": true, "timeout": 300}}
  ],
  "test_tasks": [
    {{"description": "specific test task description with concrete scenarios", "covers_features": [0], "target_files": ["test/app.test.js"], "rationale": "why this feature needs a dedicated test task"}}
  ],
  "build_timeout": 600,
  "test_timeout": 600,
  "rationale": "why these commands and steps are appropriate for this project"
}}

Important: Use only ASCII hyphen-minus characters in shell commands and flags. Never use Unicode dash punctuation in build_cmd, test_cmd, or pipeline_steps[].command.
Important: Every command string must be valid shell syntax and also JSON-safe. Do not use unescaped double quotes inside build_cmd, test_cmd, or pipeline_steps[].command. Prefer single quotes around shell arguments when quoting is needed (for example, `-destination 'platform=macOS'`).

Notes on the JSON fields:
- `pipeline_steps[].sort_order`: integer execution sequence (1, 2, 3...)
- `pipeline_steps[].required`: boolean; if true, pipeline stops on failure
- `pipeline_steps[].timeout`: integer seconds before the step is killed
- `test_tasks[].covers_features`: 0-based indices into the task list above for the feature work this dedicated test task validates
- `test_tasks[].target_files`: concrete test files expected to change for this dedicated test task
- Only include test_tasks for complex features that need dedicated test tasks

Respond ONLY with valid JSON."#
    )
}

/// Run wizard phase 7: Build & Test Configuration with Test Strategy.
///
/// Reads the current feature task list, sends the technical details from phase 3
/// and the task list to the AI provider to get recommended build/test commands,
/// validation pipeline steps, and test tasks. Writes the results to the config
/// table, inserts pipeline steps into validation_steps, and creates test tasks
/// with dependency relationships to their corresponding feature tasks.
///
/// Returns (build_cmd, test_cmd, pipeline_steps_count, test_tasks_count).
pub async fn run_wizard_phase_7(
    provider: &dyn Provider,
    prd_conn: &Connection,
    state: &mut WizardState,
    event_sink: Option<&WizardEventSink>,
) -> Result<(String, String, usize, usize)> {
    if state
        .completed_phases
        .contains(&(WizardPhase::BuildTestConfig as i32))
    {
        return Ok((String::new(), String::new(), 0, 0));
    }

    state.current_phase = WizardPhase::BuildTestConfig;
    save_wizard_state(prd_conn, state)?;

    // 1. Read existing feature tasks from the DIAL database
    let phase_conn = crate::db::get_db(None)?;
    let tasks = read_task_list(&phase_conn)?;

    // 2. Build the prompt with technical details and feature task list
    let prompt = build_build_test_config_prompt(&state.gathered_info, &tasks);

    // 3. Send to provider and parse JSON response
    let response =
        execute_wizard_prompt(provider, WizardPhase::BuildTestConfig, &prompt, event_sink).await?;
    let data = parse_json_response(
        &response,
        provider,
        WizardPhase::BuildTestConfig,
        &prompt,
        event_sink,
    )
    .await?;

    // 4. Apply config, pipeline steps, and test tasks to the database
    let phase6_task_plans = parse_phase6_task_plan_records(&state.gathered_info);
    let (build_cmd, test_cmd, steps_count, test_tasks_count) =
        apply_build_test_config_with_phase6_plan(&phase_conn, &data, &tasks, &phase6_task_plans)?;

    // 5. Store in wizard state and mark complete
    state.set_phase_data(WizardPhase::BuildTestConfig, data);
    state.mark_phase_complete(WizardPhase::BuildTestConfig);
    save_wizard_state(prd_conn, state)?;

    Ok((build_cmd, test_cmd, steps_count, test_tasks_count))
}

/// Apply the AI-recommended build/test configuration to the database.
///
/// Writes `build_cmd`, `test_cmd`, `build_timeout`, `test_timeout` to the config
/// table. If `pipeline_steps` are provided and non-empty, clears existing
/// validation_steps and inserts the new ones. If `test_tasks` are provided,
/// creates them with dependency relationships to the corresponding feature tasks.
///
/// # Arguments
/// * `conn` - Connection to the main DIAL database
/// * `config_data` - Parsed JSON response from the AI provider
/// * `feature_tasks` - Current feature tasks (id, description, priority, section)
///   used to map test task dependencies by 0-based index
///
/// Returns (build_cmd, test_cmd, pipeline_steps_count, test_tasks_count).
pub fn apply_build_test_config(
    conn: &Connection,
    config_data: &JsonValue,
    feature_tasks: &[(i64, String, i32, Option<String>)],
) -> Result<(String, String, usize, usize)> {
    apply_build_test_config_with_phase6_plan(
        conn,
        config_data,
        feature_tasks,
        &std::collections::HashMap::new(),
    )
}

fn upsert_phase_config(conn: &Connection, key: &str, value: &str) -> Result<()> {
    let normalized = if key.ends_with("_cmd") {
        let sanitized = sanitize_shell_command(&format!("config '{}'", key), value)?;
        if let Some(warning) = &sanitized.warning {
            print_warning(warning);
        }
        sanitized.value
    } else {
        value.to_string()
    };
    let now = chrono::Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO config (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
        params![key, normalized, now],
    )?;
    Ok(())
}

fn apply_build_test_config_with_phase6_plan(
    conn: &Connection,
    config_data: &JsonValue,
    feature_tasks: &[(i64, String, i32, Option<String>)],
    phase6_task_plans: &std::collections::HashMap<String, Phase6TaskPlanRecord>,
) -> Result<(String, String, usize, usize)> {
    let raw_build_cmd = config_data
        .get("build_cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let raw_test_cmd = config_data
        .get("test_cmd")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let build_cmd = sanitize_shell_command("build command", &raw_build_cmd)?;
    if let Some(warning) = &build_cmd.warning {
        print_warning(warning);
    }

    let test_cmd = sanitize_shell_command("test command", &raw_test_cmd)?;
    if let Some(warning) = &test_cmd.warning {
        print_warning(warning);
    }

    let build_timeout = config_data
        .get("build_timeout")
        .and_then(|v| v.as_i64())
        .unwrap_or(600);

    let test_timeout = config_data
        .get("test_timeout")
        .and_then(|v| v.as_i64())
        .unwrap_or(600);

    // Write config values directly to the active phase DB.
    upsert_phase_config(conn, "build_cmd", &build_cmd.value)?;
    upsert_phase_config(conn, "test_cmd", &test_cmd.value)?;
    upsert_phase_config(conn, "build_timeout", &build_timeout.to_string())?;
    upsert_phase_config(conn, "test_timeout", &test_timeout.to_string())?;

    // Insert pipeline steps if provided
    let steps_count = if let Some(steps) =
        config_data.get("pipeline_steps").and_then(|s| s.as_array())
    {
        if !steps.is_empty() {
            // Clear existing validation steps before inserting new ones
            conn.execute("DELETE FROM validation_steps", [])?;
            let mut inserted_steps = 0;

            for step in steps {
                let name = step.get("name").and_then(|v| v.as_str()).unwrap_or("step");
                let raw_command = step.get("command").and_then(|v| v.as_str()).unwrap_or("");
                let required = step
                    .get("required")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                if !required && pipeline_step_is_brittle_inline_eval(raw_command) {
                    print_warning(&format!(
                        "Skipping optional pipeline step '{}' because inline node -e commands with complex or shell-sensitive quoting are brittle across shells; prefer npm test, node --check, or a dedicated test task instead.",
                        name
                    ));
                    continue;
                }
                let command =
                    sanitize_shell_command(&format!("pipeline step '{}'", name), raw_command)?;
                if let Some(warning) = &command.warning {
                    print_warning(warning);
                }
                // Accept both "sort_order" (new) and "order" (legacy) field names
                let order = step
                    .get("sort_order")
                    .and_then(|v| v.as_i64())
                    .or_else(|| step.get("order").and_then(|v| v.as_i64()))
                    .unwrap_or(0) as i32;
                let timeout = step.get("timeout").and_then(|v| v.as_i64());

                conn.execute(
                    "INSERT INTO validation_steps (name, command, sort_order, required, timeout_secs)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        name,
                        command.value,
                        order,
                        if required { 1 } else { 0 },
                        timeout,
                    ],
                )?;
                inserted_steps += 1;
            }
            inserted_steps
        } else {
            0
        }
    } else {
        0
    };

    // Create test tasks with dependencies on feature tasks
    let test_tasks = parse_test_strategy_response(config_data);
    let mut test_tasks_count = 0;
    let existing_feature_fingerprints: std::collections::HashSet<String> = feature_tasks
        .iter()
        .map(|(_, description, _, _)| normalize_task_text_for_dedupe(description))
        .collect();
    let mut inserted_test_fingerprints = std::collections::HashSet::new();
    let existing_test_task_scopes: Vec<(
        std::collections::HashSet<String>,
        std::collections::HashSet<String>,
        String,
    )> = feature_tasks
        .iter()
        .filter(|(_, description, _, _)| task_explicitly_owns_test_coverage(description))
        .map(|(_, description, _, _)| {
            let mut existing_groups = feature_tasks
                .iter()
                .filter(|(_, candidate_description, _, _)| {
                    !task_explicitly_owns_test_coverage(candidate_description)
                        && task_covers_feature_scope(description, candidate_description)
                })
                .filter_map(|(_, candidate_description, _, _)| {
                    phase6_task_plans
                        .get(&normalize_task_text_for_dedupe(candidate_description))
                        .and_then(|plan| plan.feature_group.clone())
                })
                .collect::<std::collections::HashSet<_>>();
            if let Some(plan) = phase6_task_plans.get(&normalize_task_text_for_dedupe(description))
            {
                if let Some(feature_group) = &plan.feature_group {
                    existing_groups.insert(feature_group.clone());
                }
            }
            (
                existing_groups,
                task_path_hints(description),
                description.clone(),
            )
        })
        .collect();

    for test_task in &test_tasks {
        let mut covered_features = if test_task.covers_features.is_empty() {
            vec![test_task.depends_on_feature]
        } else {
            test_task.covers_features.clone()
        };
        covered_features.sort_unstable();
        covered_features.dedup();
        covered_features.retain(|index| *index < feature_tasks.len());
        if covered_features.is_empty() {
            continue;
        }

        let feature_id = feature_tasks[covered_features[0]].0;
        if task_is_manual_browser_verification(&test_task.description) {
            conn.execute(
                "UPDATE tasks
                 SET requires_browser_verification = 1
                 WHERE id = ?1",
                params![feature_id],
            )?;
            continue;
        }
        if covered_features
            .iter()
            .any(|index| task_explicitly_owns_test_coverage(&feature_tasks[*index].1))
        {
            continue;
        }

        let candidate_groups: std::collections::HashSet<String> = covered_features
            .iter()
            .filter_map(|index| {
                phase6_task_plans
                    .get(&normalize_task_text_for_dedupe(&feature_tasks[*index].1))
                    .and_then(|plan| plan.feature_group.clone())
            })
            .collect();
        let candidate_files = normalized_path_list(&test_task.target_files);
        let overlaps_existing_dedicated_scope = existing_test_task_scopes.iter().any(
            |(existing_groups, existing_files, existing_description)| {
                let group_overlap = !candidate_groups.is_empty()
                    && !existing_groups.is_empty()
                    && !existing_groups.is_disjoint(&candidate_groups);
                let file_overlap = !candidate_files.is_empty()
                    && !existing_files.is_empty()
                    && !existing_files.is_disjoint(&candidate_files);
                (group_overlap
                    && (file_overlap
                        || descriptions_materially_overlap(
                            existing_description,
                            &test_task.description,
                        )))
                    || (file_overlap
                        && descriptions_materially_overlap(
                            existing_description,
                            &test_task.description,
                        ))
            },
        );
        if overlaps_existing_dedicated_scope {
            continue;
        }
        if covered_features.iter().any(|feature_index| {
            let feature_description = &feature_tasks[*feature_index].1;
            feature_tasks
                .iter()
                .enumerate()
                .any(|(index, (_, description, _, _))| {
                    index != *feature_index
                        && task_covers_feature_scope(description, feature_description)
                })
        }) {
            continue;
        }

        let test_fingerprint = normalize_task_text_for_dedupe(&test_task.description);
        if existing_feature_fingerprints.contains(&test_fingerprint)
            || !inserted_test_fingerprints.insert(test_fingerprint)
        {
            continue;
        }

        let test_priority = covered_features
            .iter()
            .map(|index| feature_tasks[*index].2)
            .max()
            .unwrap_or(feature_tasks[0].2)
            + 1;

        let prd_section_id = covered_features
            .iter()
            .filter_map(|index| feature_tasks[*index].3.clone())
            .next();

        conn.execute(
            "INSERT INTO tasks (description, status, priority, prd_section_id)
             VALUES (?1, 'pending', ?2, ?3)",
            params![test_task.description, test_priority, prd_section_id],
        )?;

        let test_task_id = conn.last_insert_rowid();

        for feature_index in &covered_features {
            conn.execute(
                "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_id)
                 VALUES (?1, ?2)",
                params![test_task_id, feature_tasks[*feature_index].0],
            )?;
        }

        test_tasks_count += 1;
    }

    Ok((
        build_cmd.value,
        test_cmd.value,
        steps_count,
        test_tasks_count,
    ))
}

/// Build the phase 8 (Iteration Mode) prompt with project context and task count.
///
/// Unlike generic phases that only use `gathered_info` as prior context, phase 8
/// extracts the project name, task count, and complexity indicators from the
/// accumulated wizard state so the AI can recommend an appropriate iteration mode.
///
/// # Arguments
/// * `gathered_info` - Accumulated wizard state from phases 1-7
/// * `task_count` - Number of pending/in-progress tasks in the database
pub fn build_iteration_mode_prompt(gathered_info: &JsonValue, task_count: usize) -> String {
    build_iteration_mode_prompt_with_preference(gathered_info, task_count, None)
}

pub fn build_iteration_mode_prompt_with_preference(
    gathered_info: &JsonValue,
    task_count: usize,
    preferred_ai_cli: Option<&str>,
) -> String {
    let project_name = gathered_info
        .get("vision")
        .and_then(|v| v.get("project_name"))
        .and_then(|n| n.as_str())
        .filter(|name| !is_generic_project_name(name))
        .unwrap_or("current project");

    let complexity_context = {
        let mut parts: Vec<String> = Vec::new();

        // Extract feature count from functionality phase
        if let Some(functionality) = gathered_info.get("functionality") {
            if let Some(features) = functionality.get("mvp_features").and_then(|f| f.as_array()) {
                parts.push(format!("- MVP features: {}", features.len()));
            }
        }

        // Extract integration count from technical phase
        if let Some(technical) = gathered_info.get("technical") {
            if let Some(integrations) = technical.get("integrations").and_then(|i| i.as_array()) {
                parts.push(format!("- External integrations: {}", integrations.len()));
            }
            if let Some(constraints) = technical.get("constraints").and_then(|c| c.as_array()) {
                parts.push(format!("- Constraints: {}", constraints.len()));
            }
        }

        // Extract gap count from gap analysis phase
        if let Some(gap_analysis) = gathered_info.get("gap_analysis") {
            if let Some(gaps) = gap_analysis.get("gaps").and_then(|g| g.as_array()) {
                parts.push(format!("- Identified gaps: {}", gaps.len()));
            }
        }

        if parts.is_empty() {
            String::new()
        } else {
            format!("\n## Complexity Indicators\n{}\n", parts.join("\n"))
        }
    };

    let project_summary = build_project_summary_context(gathered_info);
    let current_cli_hint = preferred_ai_cli
        .map(|cli| {
            format!(
                "\nCurrent machine-default CLI for this wizard run: `{}`. Unless there is a strong reason otherwise, keep `ai_cli` set to this current backend so auto-run continues with the same available tool.\n",
                cli
            )
        })
        .unwrap_or_default();
    let autonomy_bias = if should_prefer_autonomous_iteration(gathered_info, task_count) {
        "\nAutonomy bias:\n- This is a small, local project with 5 or fewer pending tasks and no external integrations.\n- Unless the gathered information shows a concrete blocking risk, prefer `autonomous` so `dial auto-run` can complete the full loop without manual checkpoints.\n"
    } else {
        ""
    };

    format!(
        r#"You are recommending an iteration mode for autonomous AI development of a software project.

## Project Summary
- Project: {project_name}
- Pending tasks: {task_count}
{complexity_context}
{project_summary}
{current_cli_hint}
{autonomy_bias}
Available iteration modes:
- "autonomous": Run all tasks without stopping. Commit on pass, skip to next on failure. Best for well-specified projects with strong test coverage.
- "review_every:N": Pause for human review after every N completed tasks. Good balance of speed and oversight.
- "review_each": Pause after every single task for human approval before continuing. Best for complex, high-risk, or exploratory projects.

Consider:
1. More tasks and higher complexity → more review points
2. External integrations and constraints → more risk → more review
3. Well-defined, isolated tasks → safer for autonomous mode
4. Projects with many dependencies between tasks → benefit from review

Respond in JSON format:
{{
  "recommended_mode": "autonomous",
  "review_interval": null,
  "ai_cli": "claude",
  "subagent_timeout": 1800,
  "rationale": "why this mode is appropriate for this project"
}}

Notes:
- "recommended_mode" must be one of: "autonomous", "review_every", "review_each"
- "review_interval" should be a positive integer when mode is "review_every", null otherwise
- "ai_cli" should be "claude", "codex", "copilot", or "gemini"
- "subagent_timeout" is in seconds (default 1800 = 30 minutes)

Respond ONLY with valid JSON."#
    )
}

fn should_prefer_autonomous_iteration(gathered_info: &JsonValue, task_count: usize) -> bool {
    if task_count == 0 || task_count > 5 {
        return false;
    }

    let feature_count = gathered_info
        .get("functionality")
        .and_then(|f| f.get("mvp_features"))
        .and_then(|v| v.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    let integration_count = gathered_info
        .get("technical")
        .and_then(|t| t.get("integrations"))
        .and_then(|v| v.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    let gap_count = gathered_info
        .get("gap_analysis")
        .and_then(|g| g.get("gaps"))
        .and_then(|v| v.as_array())
        .map(|items| items.len())
        .unwrap_or(0);

    integration_count == 0 && feature_count <= 5 && gap_count <= 8
}

pub fn apply_autonomous_iteration_override(
    gathered_info: &JsonValue,
    task_count: usize,
    mode_data: &JsonValue,
) -> JsonValue {
    if !should_prefer_autonomous_iteration(gathered_info, task_count) {
        return mode_data.clone();
    }

    let raw_mode = mode_data
        .get("recommended_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("autonomous");
    if raw_mode == "autonomous" {
        return mode_data.clone();
    }

    let mut overridden = mode_data.clone();
    if let Some(object) = overridden.as_object_mut() {
        object.insert(
            "recommended_mode".to_string(),
            JsonValue::String("autonomous".to_string()),
        );
        object.insert("review_interval".to_string(), JsonValue::Null);

        let existing_rationale = object
            .get("rationale")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let override_rationale = if existing_rationale.is_empty() {
            "Small-project autonomy override: 5 or fewer pending tasks, no external integrations, and bounded complexity favor a fully autonomous loop."
                .to_string()
        } else {
            format!(
                "Small-project autonomy override: 5 or fewer pending tasks, no external integrations, and bounded complexity favor a fully autonomous loop. Original provider rationale: {}",
                existing_rationale
            )
        };
        object.insert(
            "rationale".to_string(),
            JsonValue::String(override_rationale),
        );
    }

    overridden
}

/// Run wizard phase 8: Iteration Mode.
///
/// Sends the project context and task count to the AI provider to get a
/// recommended iteration mode. Writes the results (mode, review_interval,
/// ai_cli, subagent_timeout) to the config table.
///
/// Returns the recommended mode string.
pub async fn run_wizard_phase_8(
    provider: &dyn Provider,
    prd_conn: &Connection,
    state: &mut WizardState,
    event_sink: Option<&WizardEventSink>,
) -> Result<String> {
    if state
        .completed_phases
        .contains(&(WizardPhase::IterationMode as i32))
    {
        return Ok(String::new());
    }

    state.current_phase = WizardPhase::IterationMode;
    save_wizard_state(prd_conn, state)?;

    // 1. Count pending tasks from the DIAL database
    let phase_conn = crate::db::get_db(None)?;
    let task_count: usize = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status IN ('pending', 'in_progress')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;

    let preferred_ai_cli = match provider.name() {
        "claude" | "codex" | "copilot" | "gemini" => Some(provider.name()),
        _ => None,
    };

    // 2. Build the prompt with project context and task count
    let prompt = build_iteration_mode_prompt_with_preference(
        &state.gathered_info,
        task_count,
        preferred_ai_cli,
    );

    // 3. Send to provider and parse JSON response
    let response =
        execute_wizard_prompt(provider, WizardPhase::IterationMode, &prompt, event_sink).await?;
    let data = parse_json_response(
        &response,
        provider,
        WizardPhase::IterationMode,
        &prompt,
        event_sink,
    )
    .await?;
    let data = apply_autonomous_iteration_override(&state.gathered_info, task_count, &data);

    // 4. Apply iteration mode config to the database
    let mode = apply_iteration_mode_with_preference(&phase_conn, &data, preferred_ai_cli)?;

    // 5. Store in wizard state and mark complete
    state.set_phase_data(WizardPhase::IterationMode, data);
    state.mark_phase_complete(WizardPhase::IterationMode);
    save_wizard_state(prd_conn, state)?;

    Ok(mode)
}

/// Apply the AI-recommended iteration mode to the database.
///
/// Writes `iteration_mode`, `review_interval`, `ai_cli`, and `subagent_timeout`
/// to the config table.
///
/// Returns the resolved mode string (e.g., "autonomous", "review_every:5", "review_each").
pub fn apply_iteration_mode(_conn: &Connection, mode_data: &JsonValue) -> Result<String> {
    apply_iteration_mode_with_preference(_conn, mode_data, None)
}

pub fn apply_iteration_mode_with_preference(
    _conn: &Connection,
    mode_data: &JsonValue,
    preferred_ai_cli: Option<&str>,
) -> Result<String> {
    let raw_mode = mode_data
        .get("recommended_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("autonomous");

    let review_interval = mode_data.get("review_interval").and_then(|v| v.as_u64());

    // Build the full mode string: "review_every:N" when interval is provided
    let mode = if raw_mode == "review_every" {
        let n = review_interval.unwrap_or(5);
        format!("review_every:{}", n)
    } else {
        raw_mode.to_string()
    };

    let ai_cli = preferred_ai_cli
        .or_else(|| mode_data.get("ai_cli").and_then(|v| v.as_str()))
        .unwrap_or("claude")
        .to_string();

    let subagent_timeout = mode_data
        .get("subagent_timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(1800);

    // Write config values via config_set (consistent with phase 7)
    crate::config::config_set("iteration_mode", &mode)?;
    crate::config::config_set("ai_cli", &ai_cli)?;
    crate::config::config_set("subagent_timeout", &subagent_timeout.to_string())?;

    Ok(mode)
}

/// Run wizard phase 9: Launch Summary.
///
/// This phase does NOT call an AI provider. It:
/// 1. Gathers project name from wizard state (gathered_info.vision.project_name)
/// 2. Counts pending/in_progress tasks from the DIAL database
/// 3. Reads build_cmd, test_cmd, iteration_mode, ai_cli from config
/// 4. Formats and prints a launch summary
/// 5. Writes launch_ready flag to wizard state gathered_info
///
/// Returns the launch summary for event emission.
pub fn run_wizard_phase_9(prd_conn: &Connection, state: &mut WizardState) -> Result<LaunchSummary> {
    if state
        .completed_phases
        .contains(&(WizardPhase::Launch as i32))
    {
        let launch = state
            .gathered_info
            .get("launch")
            .cloned()
            .unwrap_or_default();
        let project_name = launch
            .get("project_name")
            .and_then(|v| v.as_str())
            .or_else(|| {
                state
                    .gathered_info
                    .get("vision")
                    .and_then(|v| v.get("project_name"))
                    .and_then(|v| v.as_str())
            })
            .filter(|name| !is_generic_project_name(name))
            .unwrap_or("Current Project")
            .to_string();
        return Ok(LaunchSummary {
            project_name,
            task_count: launch
                .get("task_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            build_cmd: launch
                .get("build_cmd")
                .and_then(|v| v.as_str())
                .unwrap_or("(not set)")
                .to_string(),
            test_cmd: launch
                .get("test_cmd")
                .and_then(|v| v.as_str())
                .unwrap_or("(not set)")
                .to_string(),
            iteration_mode: launch
                .get("iteration_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("(not set)")
                .to_string(),
            ai_cli: launch
                .get("ai_cli")
                .and_then(|v| v.as_str())
                .unwrap_or("(not set)")
                .to_string(),
        });
    }

    state.current_phase = WizardPhase::Launch;
    save_wizard_state(prd_conn, state)?;

    // 1. Extract project name from gathered_info
    let project_name = state
        .gathered_info
        .get("vision")
        .and_then(|v| v.get("project_name"))
        .and_then(|v| v.as_str())
        .filter(|name| !is_generic_project_name(name))
        .unwrap_or("Current Project")
        .to_string();

    // 2. Count pending tasks from the DIAL database
    let phase_conn = crate::db::get_db(None)?;
    let task_count: usize = phase_conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE status IN ('pending', 'in_progress')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize;

    // 3. Read config values (treat empty strings as not set)
    let not_set = "(not set)".to_string();
    let build_cmd = crate::config::config_get("build_cmd")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| not_set.clone());
    let test_cmd = crate::config::config_get("test_cmd")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| not_set.clone());
    let iteration_mode = crate::config::config_get("iteration_mode")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| not_set.clone());
    let ai_cli = crate::config::config_get("ai_cli")?
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| not_set);

    // 4. Write launch_ready flag to wizard state
    let launch_data = serde_json::json!({
        "launch_ready": true,
        "project_name": project_name,
        "task_count": task_count,
        "build_cmd": build_cmd,
        "test_cmd": test_cmd,
        "iteration_mode": iteration_mode,
        "ai_cli": ai_cli,
    });

    state.set_phase_data(WizardPhase::Launch, launch_data);
    state.mark_phase_complete(WizardPhase::Launch);
    save_wizard_state(prd_conn, state)?;

    Ok(LaunchSummary {
        project_name,
        task_count,
        build_cmd,
        test_cmd,
        iteration_mode,
        ai_cli,
    })
}

/// A specificity rating for a PRD section from Phase 4 gap analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionRating {
    pub section: String,
    pub rating: String,
    pub issues: Vec<String>,
}

/// A rewritten section from Phase 4 specificity check.
#[derive(Debug, Clone, PartialEq)]
pub struct RewrittenSection {
    pub section: String,
    pub original: String,
    pub rewritten: String,
}

/// Parse specificity ratings and rewritten sections from a Phase 4 gap analysis response.
///
/// Extracts `section_ratings` and `rewritten_sections` arrays from the JSON response.
/// Returns empty vectors if the fields are missing (backward compatible with
/// older Phase 4 responses that lack specificity data).
pub fn parse_specificity_response(data: &JsonValue) -> (Vec<SectionRating>, Vec<RewrittenSection>) {
    let ratings = data
        .get("section_ratings")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let section = item.get("section")?.as_str()?.to_string();
                    let rating = item.get("rating")?.as_str()?.to_string();
                    let issues = item
                        .get("issues")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|i| i.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(SectionRating {
                        section,
                        rating,
                        issues,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let rewrites = data
        .get("rewritten_sections")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let section = item.get("section")?.as_str()?.to_string();
                    let original = item
                        .get("original")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let rewritten = item
                        .get("rewritten")
                        .or_else(|| item.get("rewrite_summary"))
                        .and_then(|v| v.as_str())?
                        .to_string();
                    Some(RewrittenSection {
                        section,
                        original,
                        rewritten,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    (ratings, rewrites)
}

/// Apply rewritten sections from Phase 4 specificity check to the PRD database.
///
/// Looks up sections by title and updates their content with the rewritten text.
/// Returns the number of sections successfully updated.
pub fn apply_specificity_rewrites(
    conn: &Connection,
    rewrites: &[RewrittenSection],
) -> Result<usize> {
    let mut updated = 0;
    for rewrite in rewrites {
        // Look up the section by title
        let section_id: Option<String> = conn
            .query_row(
                "SELECT section_id FROM sections WHERE title = ?1",
                params![rewrite.section],
                |row| row.get(0),
            )
            .ok();

        if let Some(sid) = section_id {
            let word_count = rewrite.rewritten.split_whitespace().count() as i32;
            let rows = conn.execute(
                "UPDATE sections SET content = ?1, word_count = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%S', 'now')
                 WHERE section_id = ?3",
                params![rewrite.rewritten, word_count, sid],
            )?;
            if rows > 0 {
                updated += 1;
            }
        }
    }
    Ok(updated)
}

/// Parse task sizing analysis data from a Phase 6 task review response.
///
/// Extracts `splits`, `rewrites`, `merges`, and `sizing_summary` from the JSON response.
/// Returns defaults if any fields are missing (backward compatible with older Phase 6 responses).
pub fn parse_sizing_response(
    data: &JsonValue,
) -> (
    Vec<TaskSplitRecord>,
    Vec<TaskRewriteRecord>,
    Vec<TaskMergeRecord>,
    SizingSummary,
) {
    let splits: Vec<TaskSplitRecord> = data
        .get("splits")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let original = item.get("original")?.as_str()?.to_string();
                    let into = item
                        .get("into")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|i| i.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let reason = item
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(TaskSplitRecord {
                        original,
                        into,
                        reason,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let rewrites: Vec<TaskRewriteRecord> = data
        .get("rewrites")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let original = item.get("original")?.as_str()?.to_string();
                    let rewritten = item.get("rewritten")?.as_str()?.to_string();
                    let reason = item
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(TaskRewriteRecord {
                        original,
                        rewritten,
                        reason,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let merges: Vec<TaskMergeRecord> = data
        .get("merges")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let merged = item
                        .get("merged")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|i| i.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let into = item.get("into")?.as_str()?.to_string();
                    let reason = item
                        .get("reason")
                        .and_then(|r| r.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(TaskMergeRecord {
                        merged,
                        into,
                        reason,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Parse sizing_summary, counting from individual task sizes as fallback
    let summary = if let Some(ss) = data.get("sizing_summary") {
        SizingSummary {
            small: ss.get("S").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            medium: ss.get("M").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            large: ss.get("L").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            xl: ss.get("XL").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            total_splits: ss
                .get("total_splits")
                .and_then(|v| v.as_u64())
                .unwrap_or(splits.len() as u64) as usize,
            total_rewrites: ss
                .get("total_rewrites")
                .and_then(|v| v.as_u64())
                .unwrap_or(rewrites.len() as u64) as usize,
            total_merges: ss
                .get("total_merges")
                .and_then(|v| v.as_u64())
                .unwrap_or(merges.len() as u64) as usize,
        }
    } else {
        // Fallback: count sizes from task entries
        let mut small = 0usize;
        let mut medium = 0usize;
        let mut large = 0usize;
        let mut xl = 0usize;
        if let Some(tasks) = data.get("tasks").and_then(|t| t.as_array()) {
            for task in tasks {
                match task.get("size").and_then(|s| s.as_str()).unwrap_or("M") {
                    "S" => small += 1,
                    "M" => medium += 1,
                    "L" => large += 1,
                    "XL" => xl += 1,
                    _ => medium += 1,
                }
            }
        }
        SizingSummary {
            small,
            medium,
            large,
            xl,
            total_splits: splits.len(),
            total_rewrites: rewrites.len(),
            total_merges: merges.len(),
        }
    };

    (splits, rewrites, merges, summary)
}

/// Parse test strategy data from a Phase 7 build/test config response.
///
/// Extracts `test_tasks` array from the JSON response. Each test task has a
/// description, one or more `covers_features` indices (0-based into the task list),
/// optional concrete `target_files`, and a rationale.
///
/// Returns empty vector if the field is missing (backward compatible with
/// older Phase 7 responses that lack test strategy data).
pub fn parse_test_strategy_response(data: &JsonValue) -> Vec<TestTaskRecord> {
    data.get("test_tasks")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let description = item.get("description")?.as_str()?.to_string();
                    let explicit_depends_on_feature = item
                        .get("depends_on_feature")
                        .and_then(|v| v.as_u64())
                        .map(|value| value as usize);
                    let covers_features = item
                        .get("covers_features")
                        .and_then(|value| value.as_array())
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|value| value.as_u64().map(|index| index as usize))
                                .collect::<Vec<_>>()
                        })
                        .filter(|values| !values.is_empty())
                        .unwrap_or_else(|| vec![explicit_depends_on_feature.unwrap_or(0)]);
                    let depends_on_feature = explicit_depends_on_feature
                        .or_else(|| covers_features.first().copied())
                        .unwrap_or(0);
                    let target_files = item
                        .get("target_files")
                        .and_then(|value| value.as_array())
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(|value| value.as_str())
                                .map(|value| value.trim().to_string())
                                .filter(|value| !value.is_empty())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let rationale = item
                        .get("rationale")
                        .and_then(|r| r.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(TestTaskRecord {
                        description,
                        depends_on_feature,
                        covers_features,
                        target_files,
                        rationale,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::time::sleep;

    struct TestProvider {
        responses: Mutex<Vec<String>>,
        name: &'static str,
    }

    impl TestProvider {
        fn new(name: &'static str, responses: Vec<String>) -> Self {
            Self {
                responses: Mutex::new(responses),
                name,
            }
        }
    }

    #[async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            self.name
        }

        async fn execute(
            &self,
            _request: ProviderRequest,
        ) -> Result<crate::provider::ProviderResponse> {
            let output = self
                .responses
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .remove(0);

            Ok(crate::provider::ProviderResponse {
                output,
                success: true,
                exit_code: Some(0),
                usage: None,
                model: Some("test-model".to_string()),
                duration_secs: Some(0.1),
            })
        }

        async fn is_available(&self) -> bool {
            true
        }
    }

    struct DelayedTestProvider {
        response: String,
        delay: Duration,
        name: &'static str,
    }

    impl DelayedTestProvider {
        fn new(name: &'static str, response: &str, delay: Duration) -> Self {
            Self {
                response: response.to_string(),
                delay,
                name,
            }
        }
    }

    #[async_trait]
    impl Provider for DelayedTestProvider {
        fn name(&self) -> &str {
            self.name
        }

        async fn execute(
            &self,
            _request: ProviderRequest,
        ) -> Result<crate::provider::ProviderResponse> {
            sleep(self.delay).await;

            Ok(crate::provider::ProviderResponse {
                output: self.response.clone(),
                success: true,
                exit_code: Some(0),
                usage: None,
                model: Some("test-model".to_string()),
                duration_secs: Some(self.delay.as_secs_f64()),
            })
        }

        async fn is_available(&self) -> bool {
            true
        }
    }

    fn recording_sink(events: Arc<Mutex<Vec<Event>>>) -> WizardEventSink {
        Arc::new(move |event| {
            events
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push(event);
        })
    }

    // --- Prompt Content Tests ---

    #[test]
    fn test_wizard_work_dir_uses_temp_directory() {
        assert_eq!(
            wizard_work_dir(),
            std::env::temp_dir().to_string_lossy().to_string()
        );
    }

    #[test]
    fn test_vision_phase_schema_requires_expected_fields() {
        let schema = wizard_phase_output_schema(WizardPhase::Vision).unwrap();
        let value: JsonValue = serde_json::from_str(&schema).unwrap();
        let required = value["required"].as_array().unwrap();
        assert!(required.iter().any(|item| item == "project_name"));
        assert!(required.iter().any(|item| item == "target_users"));
        assert_eq!(value["properties"]["project_name"]["minLength"], json!(3));
        assert_eq!(value["properties"]["target_users"]["minItems"], json!(1));
    }

    #[test]
    fn test_vision_prompt_requires_concrete_project_name() {
        let state = WizardState::new("mvp");
        let prompt = build_phase_prompt(WizardPhase::Vision, &state, None);

        assert!(prompt.contains("must be a concrete product name"));
        assert!(prompt.contains("Do not use generic names"));
    }

    #[test]
    fn test_task_review_prompt_forbids_placeholder_language() {
        let tasks = vec![(1, "Task".to_string(), 1, None)];
        let prompt = build_task_review_prompt(&tasks, &json!({}));

        assert!(prompt.contains("NO PLACEHOLDERS"));
        assert!(prompt.contains("as defined in task 2"));
    }

    #[test]
    fn test_build_project_summary_omits_generic_project_name() {
        let gathered = json!({
            "vision": {
                "project_name": "unknown",
                "problem_statement": "Coordinate volunteer rehearsals and music planning."
            }
        });

        let summary = build_project_summary_context(&gathered);
        assert!(!summary.contains("- Project: unknown"));
        assert!(summary.contains("- Problem: Coordinate volunteer rehearsals"));
    }

    #[test]
    fn test_extract_source_requirements_captures_structured_sections() {
        let doc = r#"
# Scenario

## Goal

Validate the complete DIAL loop on a small native Windows project.

## Requested Outcome

1. Add note status formatting:
   - input status `todo` renders `[ ]`
   - input status `done` renders `[x]`

## Hard Constraints

- Keep the implementation within `src/noteFormatter.js`, `src/cli.js`, and `test/noteFormatter.test.js`
- Prefer 3 to 5 implementation tasks total

## Acceptance Checks

- `npm test` passes
- `node src/cli.js` accepts JSON from stdin and prints the formatted note
"#;

        let extracted = extract_source_requirements(doc);

        assert!(extracted["goal"]
            .as_str()
            .unwrap()
            .contains("Validate the complete DIAL loop"));
        assert!(extracted["requested_outcome"]
            .as_str()
            .unwrap()
            .contains("input status `todo` renders `[ ]`"));
        assert!(extracted["hard_constraints"]
            .as_str()
            .unwrap()
            .contains("Prefer 3 to 5 implementation tasks total"));
        assert!(extracted["acceptance_checks"]
            .as_str()
            .unwrap()
            .contains("`node src/cli.js` accepts JSON from stdin"));
    }

    #[test]
    fn test_vision_prompt_marks_existing_doc_as_source_of_truth() {
        let state = WizardState::new("spec");
        let prompt = build_phase_prompt(
            WizardPhase::Vision,
            &state,
            Some("## Requested Outcome\n- input status `todo` renders `[ ]`"),
        );

        assert!(prompt.contains("Source-of-Truth Rules"));
        assert!(prompt.contains(
            "Treat the existing document as the authoritative source of scope and acceptance"
        ));
        assert!(prompt.contains("Preserve exact literal values"));
    }

    #[test]
    fn test_collect_phase_quality_issues_flags_placeholder_task_language() {
        let data = json!({
            "tasks": [
                {
                    "description": "Add support for the second entity as defined in task 2",
                    "priority": 1,
                    "spec_section": "1.2",
                    "depends_on": [],
                    "acceptance_criteria": ["Render the second entity row in the editor"],
                    "requires_browser_verification": false,
                    "rationale": "Implements placeholder feature name",
                    "size": "M"
                }
            ],
            "removed": [],
            "added": [],
            "splits": [],
            "rewrites": [],
            "merges": [],
            "sizing_summary": {
                "S": 0, "M": 1, "L": 0, "XL": 0,
                "total_splits": 0, "total_rewrites": 0, "total_merges": 0
            }
        });

        let issues = collect_phase_quality_issues(WizardPhase::TaskReview, &data);
        assert!(!issues.is_empty());
        assert!(issues
            .iter()
            .any(|issue| issue.contains("tasks[0].description")));
    }

    #[test]
    fn test_placeholder_detection_allows_legitimate_feature_name_prose() {
        assert!(!has_placeholder_language(
            "User enters feature names and descriptions one at a time"
        ));
    }

    #[test]
    fn test_placeholder_detection_allows_comparison_operators() {
        assert!(!has_placeholder_language(
            "Set is_valid to true when content length is >= 50 characters and false when content length is < 50 characters."
        ));
    }

    #[test]
    fn test_placeholder_detection_still_flags_named_angle_placeholders() {
        assert!(has_placeholder_language(
            "Run `launchpad export <project-name>` after validation passes."
        ));
        assert!(has_placeholder_language(
            "Store the selection under <id> before continuing."
        ));
    }

    #[test]
    fn test_generate_quality_allows_angle_bracket_cli_examples() {
        let data = json!({
            "sections": [
                {
                    "title": "MVP Features",
                    "content": "Run `launchpad export <project-name>` to write the final Markdown file after validation passes."
                }
            ],
            "terminology": []
        });

        let issues = collect_phase_quality_issues(WizardPhase::Generate, &data);
        assert!(issues.is_empty(), "unexpected generate issues: {issues:?}");
    }

    #[test]
    fn test_generate_quality_allows_placeholder_terms_in_explanatory_prose() {
        let data = json!({
            "sections": [
                {
                    "title": "Validation",
                    "content": "Reject answers that only contain placeholder strings such as TBD, TODO, or lorem ipsum so users cannot save incomplete PRD sections."
                }
            ],
            "terminology": []
        });

        let issues = collect_phase_quality_issues(WizardPhase::Generate, &data);
        assert!(issues.is_empty(), "unexpected generate issues: {issues:?}");
    }

    #[test]
    fn test_generate_quality_rejects_short_todo_sections() {
        let data = json!({
            "sections": [
                {
                    "title": "Validation",
                    "content": "TODO"
                }
            ],
            "terminology": []
        });

        let issues = collect_phase_quality_issues(WizardPhase::Generate, &data);
        assert!(
            issues
                .iter()
                .any(|issue| issue.contains("placeholder language")),
            "expected placeholder issue, got {issues:?}"
        );
    }

    #[tokio::test]
    async fn test_parse_json_response_retries_generic_vision_output() {
        let provider = TestProvider::new(
            "copilot",
            vec![serde_json::to_string(&json!({
                "project_name": "ChoirCue",
                "elevator_pitch": "ChoirCue helps worship teams schedule rehearsals and share service plans.",
                "problem_statement": "Volunteer music teams struggle to coordinate rehearsals, song assignments, and service notes without losing changes in scattered group chats.",
                "target_users": ["worship leaders", "choir members"],
                "success_criteria": ["Rehearsal plans stay in one shared timeline", "Song assignments update without manual follow-up"],
                "scope_exclusions": ["full church accounting", "livestream production control"]
            }))
            .unwrap()],
        );

        let initial = serde_json::to_string(&json!({
            "project_name": "unknown",
            "elevator_pitch": "A project for users.",
            "problem_statement": "Solve TBD issues for the app.",
            "target_users": ["some users"],
            "success_criteria": ["successful outcome"],
            "scope_exclusions": ["various extras"]
        }))
        .unwrap();

        let parsed = parse_json_response(
            &initial,
            &provider,
            WizardPhase::Vision,
            "Return valid JSON for phase 1.",
            None,
        )
        .await
        .unwrap();

        assert_eq!(parsed["project_name"], json!("ChoirCue"));
        assert_eq!(parsed["target_users"][0], json!("worship leaders"));
    }

    #[tokio::test]
    async fn test_execute_wizard_prompt_emits_heartbeat_for_slow_provider() {
        let provider = DelayedTestProvider::new(
            "codex",
            r#"{"project_name":"Signal","elevator_pitch":"Signal helps teams organize launches.","problem_statement":"Teams lose track of launch tasks across channels and docs.","target_users":["product teams"],"success_criteria":["Launch work stays visible"],"scope_exclusions":["roadmap planning"]}"#,
            Duration::from_millis(35),
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = recording_sink(events.clone());

        let _ = execute_wizard_prompt_with_heartbeat(
            &provider,
            WizardPhase::Vision,
            "Return JSON.",
            Some(&sink),
            Duration::from_millis(10),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

        let heartbeats: Vec<Event> = events
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .filter(|event| matches!(event, Event::WizardHeartbeat { .. }))
            .cloned()
            .collect();

        assert!(
            !heartbeats.is_empty(),
            "expected at least one heartbeat event for a slow provider"
        );
    }

    #[tokio::test]
    async fn test_execute_wizard_prompt_skips_heartbeat_for_fast_provider() {
        let provider = DelayedTestProvider::new(
            "codex",
            r#"{"project_name":"Signal","elevator_pitch":"Signal helps teams organize launches.","problem_statement":"Teams lose track of launch tasks across channels and docs.","target_users":["product teams"],"success_criteria":["Launch work stays visible"],"scope_exclusions":["roadmap planning"]}"#,
            Duration::from_millis(5),
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = recording_sink(events.clone());

        let _ = execute_wizard_prompt_with_heartbeat(
            &provider,
            WizardPhase::Vision,
            "Return JSON.",
            Some(&sink),
            Duration::from_millis(20),
            Duration::from_millis(20),
        )
        .await
        .unwrap();

        assert!(
            events
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .iter()
                .all(|event| !matches!(event, Event::WizardHeartbeat { .. })),
            "did not expect heartbeat events for a fast provider"
        );
    }

    #[tokio::test]
    async fn test_execute_wizard_prompt_never_emits_launch_heartbeat() {
        let provider = DelayedTestProvider::new("codex", "{}", Duration::from_millis(35));
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = recording_sink(events.clone());

        let _ = execute_wizard_prompt_with_heartbeat(
            &provider,
            WizardPhase::Launch,
            "No-op launch prompt.",
            Some(&sink),
            Duration::from_millis(10),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

        assert!(
            events
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .iter()
                .all(|event| !matches!(event, Event::WizardHeartbeat { .. })),
            "launch should never emit heartbeat events"
        );
    }

    #[test]
    fn test_task_review_prompt_contains_sizing_section() {
        let tasks = vec![
            (
                1,
                "Build auth system".to_string(),
                1,
                Some("1.1".to_string()),
            ),
            (2, "Set up database".to_string(), 2, None),
        ];
        let gathered = json!({});
        let prompt = build_task_review_prompt(&tasks, &gathered);

        assert!(prompt.contains("TASK SIZING ANALYSIS"));
        assert!(prompt.contains("SCOPE"));
        assert!(prompt.contains("SPECIFICITY"));
        assert!(prompt.contains("TESTABILITY"));
    }

    #[test]
    fn test_task_review_prompt_requires_acceptance_and_browser_metadata() {
        let tasks = vec![(1, "Task".to_string(), 1, None)];
        let prompt = build_task_review_prompt(&tasks, &json!({}));

        assert!(prompt.contains("ACCEPTANCE CRITERIA"));
        assert!(prompt.contains("BROWSER VERIFICATION"));
        assert!(prompt.contains("TASK KIND"));
        assert!(prompt.contains("FEATURE GROUP"));
        assert!(prompt.contains("COVERAGE OWNERSHIP"));
        assert!(prompt.contains("\"acceptance_criteria\""));
        assert!(prompt.contains("\"requires_browser_verification\""));
        assert!(prompt.contains("\"task_kind\""));
        assert!(prompt.contains("\"feature_group\""));
        assert!(prompt.contains("\"coverage_mode\""));
    }

    #[test]
    fn test_task_review_prompt_contains_split_instructions() {
        let tasks = vec![(1, "Task".to_string(), 1, None)];
        let prompt = build_task_review_prompt(&tasks, &json!({}));

        assert!(prompt.contains("SPLIT"));
        assert!(prompt.contains("more than 3 files"));
        assert!(prompt.contains("dependency relationships"));
    }

    #[test]
    fn test_task_review_prompt_contains_rewrite_instructions() {
        let tasks = vec![(1, "Task".to_string(), 1, None)];
        let prompt = build_task_review_prompt(&tasks, &json!({}));

        assert!(prompt.contains("REWRITE"));
        assert!(prompt.contains("BAD: \"Build auth system\""));
        assert!(prompt.contains("GOOD: \"Add bcrypt password hashing"));
    }

    #[test]
    fn test_task_review_prompt_contains_merge_instructions() {
        let tasks = vec![(1, "Task".to_string(), 1, None)];
        let prompt = build_task_review_prompt(&tasks, &json!({}));

        assert!(prompt.contains("MERGE"));
        assert!(prompt.contains("too small for a separate iteration"));
    }

    #[test]
    fn test_task_review_prompt_contains_size_labels() {
        let tasks = vec![(1, "Task".to_string(), 1, None)];
        let prompt = build_task_review_prompt(&tasks, &json!({}));

        assert!(prompt.contains("[S]mall"));
        assert!(prompt.contains("[M]edium"));
        assert!(prompt.contains("[L]arge"));
        assert!(prompt.contains("[XL]needs-review"));
    }

    #[test]
    fn test_task_review_prompt_json_format_includes_sizing_fields() {
        let tasks = vec![(1, "Task".to_string(), 1, None)];
        let prompt = build_task_review_prompt(&tasks, &json!({}));

        assert!(prompt.contains("\"size\": \"S\""));
        assert!(prompt.contains("\"acceptance_criteria\""));
        assert!(prompt.contains("\"requires_browser_verification\""));
        assert!(prompt.contains("\"task_kind\": \"feature\""));
        assert!(prompt.contains("\"feature_group\": \"auth-login\""));
        assert!(prompt.contains("\"coverage_mode\": \"inline\""));
        assert!(prompt.contains("\"splits\""));
        assert!(prompt.contains("\"rewrites\""));
        assert!(prompt.contains("\"merges\""));
        assert!(prompt.contains("\"sizing_summary\""));
    }

    #[test]
    fn test_task_review_schema_requires_acceptance_and_browser_fields() {
        let schema = wizard_phase_output_schema(WizardPhase::TaskReview).unwrap();
        let value: JsonValue = serde_json::from_str(&schema).unwrap();
        let required = value["properties"]["tasks"]["items"]["required"]
            .as_array()
            .unwrap();

        assert!(required.iter().any(|item| item == "acceptance_criteria"));
        assert!(required
            .iter()
            .any(|item| item == "requires_browser_verification"));
        assert!(required.iter().any(|item| item == "task_kind"));
        assert!(required.iter().any(|item| item == "feature_group"));
        assert!(required.iter().any(|item| item == "coverage_mode"));
        assert_eq!(
            value["properties"]["tasks"]["items"]["properties"]["acceptance_criteria"]["minItems"],
            json!(1)
        );
        assert_eq!(
            value["properties"]["tasks"]["items"]["properties"]["requires_browser_verification"]
                ["type"],
            json!("boolean")
        );
        assert_eq!(
            value["properties"]["tasks"]["items"]["properties"]["task_kind"]["enum"],
            json!([
                "feature",
                "test",
                "config",
                "docs",
                "verification",
                "refactor"
            ])
        );
        assert_eq!(
            value["properties"]["tasks"]["items"]["properties"]["coverage_mode"]["enum"],
            json!(["inline", "dedicated", "none"])
        );
    }

    #[test]
    fn test_task_review_prompt_uses_compact_summary_context() {
        let tasks = vec![(
            1,
            "Implement: Problem".to_string(),
            1,
            Some("1".to_string()),
        )];
        let gathered = json!({
            "vision": {
                "project_name": "WizardTestProject",
                "problem_statement": "A fairly long problem statement that should be summarized instead of dumping the entire gathered_info blob back into the prompt."
            },
            "functionality": {
                "mvp_features": [{"name": "Import specs"}]
            },
            "generate": {
                "sections": [
                    {"title": "Problem", "content": "Long markdown content for the problem section"},
                    {"title": "Scope", "content": "Long markdown content for the scope section"}
                ]
            }
        });

        let prompt = build_task_review_prompt(&tasks, &gathered);

        assert!(prompt.contains("Project Summary"));
        assert!(prompt.contains("PRD Section Outline"));
        assert!(!prompt.contains("Full PRD Context"));
    }

    #[test]
    fn test_task_review_prompt_carries_source_requirements_forward() {
        let tasks = vec![(
            1,
            "Implement: Features".to_string(),
            1,
            Some("3".to_string()),
        )];
        let gathered = json!({
            "source_requirements": {
                "requested_outcome": "1. Add note status formatting:\n- input status `todo` renders `[ ]`\n- input status `done` renders `[x]`",
                "hard_constraints": "- Prefer 3 to 5 implementation tasks total",
                "acceptance_checks": "- `node src/cli.js` accepts JSON from stdin and prints the formatted note"
            }
        });

        let prompt = build_task_review_prompt(&tasks, &gathered);

        assert!(prompt.contains("Source Requirements (authoritative)"));
        assert!(prompt.contains("input status `todo` renders `[ ]`"));
        assert!(prompt.contains("Prefer 3 to 5 implementation tasks total"));
        assert!(prompt.contains("Do not create duplicate tasks"));
        assert!(prompt.contains("Do not introduce alternate contracts"));
    }

    #[test]
    fn test_build_test_config_prompt_carries_source_requirements_forward() {
        let tasks = vec![(
            1,
            "Implement CLI stdin flow".to_string(),
            1,
            Some("3".to_string()),
        )];
        let gathered = json!({
            "source_requirements": {
                "hard_constraints": "- Keep `npm test` as the test command",
                "acceptance_checks": "- Invalid JSON on stdin causes a non-zero exit code and a human-readable error"
            },
            "technical": {
                "platform": {
                    "languages": ["JavaScript"],
                    "frameworks": [],
                    "database": "",
                    "hosting": "local CLI"
                }
            }
        });

        let prompt = build_build_test_config_prompt(&gathered, &tasks);

        assert!(prompt.contains("Source Requirements (authoritative)"));
        assert!(prompt.contains("Keep `npm test` as the test command"));
        assert!(prompt.contains("Invalid JSON on stdin causes a non-zero exit code"));
        assert!(prompt.contains("Preserve exact commands, file paths, stdin/stdout behavior"));
    }

    #[test]
    fn test_build_test_config_prompt_uses_phase6_coverage_metadata() {
        let tasks = vec![
            (
                1,
                "Implement compact mode settings".to_string(),
                1,
                Some("3".to_string()),
            ),
            (
                2,
                "Add compact mode tests".to_string(),
                2,
                Some("3".to_string()),
            ),
        ];
        let gathered = json!({
            "task_review": {
                "tasks": [
                    {
                        "description": "Implement compact mode settings",
                        "task_kind": "feature",
                        "feature_group": "compact-mode-settings",
                        "coverage_mode": "inline"
                    },
                    {
                        "description": "Add compact mode tests",
                        "task_kind": "test",
                        "feature_group": "compact-mode-settings",
                        "coverage_mode": "dedicated"
                    }
                ]
            }
        });

        let prompt = build_build_test_config_prompt(&gathered, &tasks);

        assert!(prompt.contains("kind: feature | coverage: inline | group: compact-mode-settings"));
        assert!(prompt.contains("kind: test | coverage: dedicated | group: compact-mode-settings"));
        assert!(prompt.contains("At most one dedicated test task may own a given `group`."));
        assert!(prompt.contains("\"covers_features\""));
        assert!(prompt.contains("\"target_files\""));
    }

    #[test]
    fn test_task_review_prompt_xl_must_be_split() {
        let tasks = vec![(1, "Task".to_string(), 1, None)];
        let prompt = build_task_review_prompt(&tasks, &json!({}));

        assert!(prompt.contains("Any task sized [XL] MUST be split"));
    }

    // --- Sizing Response Parsing Tests ---

    #[test]
    fn test_parse_sizing_response_full() {
        let data = json!({
            "tasks": [
                {"description": "task1", "size": "S"},
                {"description": "task2", "size": "M"},
                {"description": "task3", "size": "L"},
            ],
            "splits": [
                {
                    "original": "Build entire auth system",
                    "into": ["Add password hashing", "Add session tokens", "Add login endpoint"],
                    "reason": "Touches too many files"
                }
            ],
            "rewrites": [
                {
                    "original": "Set up database",
                    "rewritten": "Create users table with id, email, password_hash columns",
                    "reason": "Original was vague"
                }
            ],
            "merges": [
                {
                    "merged": ["Update .gitignore", "Add .env.example"],
                    "into": "Set up project config files (.gitignore, .env.example)",
                    "reason": "Both are trivial config changes"
                }
            ],
            "sizing_summary": {
                "S": 5, "M": 3, "L": 1, "XL": 0,
                "total_splits": 1, "total_rewrites": 1, "total_merges": 1
            }
        });

        let (splits, rewrites, merges, summary) = parse_sizing_response(&data);

        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].original, "Build entire auth system");
        assert_eq!(splits[0].into.len(), 3);
        assert_eq!(splits[0].into[0], "Add password hashing");
        assert_eq!(splits[0].reason, "Touches too many files");

        assert_eq!(rewrites.len(), 1);
        assert_eq!(rewrites[0].original, "Set up database");
        assert_eq!(
            rewrites[0].rewritten,
            "Create users table with id, email, password_hash columns"
        );

        assert_eq!(merges.len(), 1);
        assert_eq!(merges[0].merged.len(), 2);
        assert_eq!(
            merges[0].into,
            "Set up project config files (.gitignore, .env.example)"
        );

        assert_eq!(summary.small, 5);
        assert_eq!(summary.medium, 3);
        assert_eq!(summary.large, 1);
        assert_eq!(summary.xl, 0);
        assert_eq!(summary.total_splits, 1);
        assert_eq!(summary.total_rewrites, 1);
        assert_eq!(summary.total_merges, 1);
    }

    #[test]
    fn test_parse_sizing_response_empty() {
        let data = json!({
            "tasks": [],
            "removed": [],
            "added": []
        });

        let (splits, rewrites, merges, summary) = parse_sizing_response(&data);

        assert!(splits.is_empty());
        assert!(rewrites.is_empty());
        assert!(merges.is_empty());
        assert_eq!(summary.small, 0);
        assert_eq!(summary.medium, 0);
        assert_eq!(summary.large, 0);
        assert_eq!(summary.xl, 0);
        assert_eq!(summary.total_splits, 0);
        assert_eq!(summary.total_rewrites, 0);
        assert_eq!(summary.total_merges, 0);
    }

    #[test]
    fn test_parse_sizing_response_fallback_counts_from_tasks() {
        let data = json!({
            "tasks": [
                {"description": "t1", "size": "S"},
                {"description": "t2", "size": "S"},
                {"description": "t3", "size": "M"},
                {"description": "t4", "size": "L"},
                {"description": "t5", "size": "XL"},
            ],
            "splits": [
                {"original": "big task", "into": ["a", "b"], "reason": "too big"}
            ]
        });

        let (splits, _rewrites, _merges, summary) = parse_sizing_response(&data);

        // Fallback: counts from individual task size fields
        assert_eq!(summary.small, 2);
        assert_eq!(summary.medium, 1);
        assert_eq!(summary.large, 1);
        assert_eq!(summary.xl, 1);
        assert_eq!(summary.total_splits, 1);
        assert_eq!(splits.len(), 1);
    }

    #[test]
    fn test_parse_sizing_response_unknown_size_defaults_to_medium() {
        let data = json!({
            "tasks": [
                {"description": "t1", "size": "unknown"},
                {"description": "t2"},
            ]
        });

        let (_splits, _rewrites, _merges, summary) = parse_sizing_response(&data);

        // Both should count as medium (fallback)
        assert_eq!(summary.medium, 2);
    }

    #[test]
    fn test_parse_sizing_response_multiple_splits() {
        let data = json!({
            "tasks": [],
            "splits": [
                {"original": "auth", "into": ["hash", "token", "login"], "reason": "too big"},
                {"original": "api", "into": ["routes", "handlers"], "reason": "multi-concern"}
            ]
        });

        let (splits, _, _, summary) = parse_sizing_response(&data);

        assert_eq!(splits.len(), 2);
        assert_eq!(splits[0].into.len(), 3);
        assert_eq!(splits[1].into.len(), 2);
        assert_eq!(summary.total_splits, 2);
    }

    #[test]
    fn test_parse_sizing_response_split_missing_reason() {
        let data = json!({
            "tasks": [],
            "splits": [
                {"original": "big task", "into": ["a", "b"]}
            ]
        });

        let (splits, _, _, _) = parse_sizing_response(&data);
        assert_eq!(splits.len(), 1);
        assert_eq!(splits[0].reason, "");
    }

    // --- apply_task_review Tests (with sizing fields) ---

    #[test]
    fn test_apply_task_review_with_size_field() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (
                id INTEGER PRIMARY KEY,
                description TEXT NOT NULL,
                status TEXT DEFAULT 'pending',
                priority INTEGER DEFAULT 5,
                blocked_by TEXT,
                spec_section_id INTEGER,
                prd_section_id TEXT,
                acceptance_criteria_json TEXT,
                requires_browser_verification INTEGER DEFAULT 0,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                started_at TEXT,
                completed_at TEXT,
                total_attempts INTEGER DEFAULT 0,
                total_failures INTEGER DEFAULT 0,
                last_failure_at TEXT
            );
            CREATE TABLE task_dependencies (
                task_id INTEGER NOT NULL,
                depends_on_id INTEGER NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (task_id, depends_on_id)
            );
            INSERT INTO tasks (description, status, priority) VALUES ('old task', 'pending', 1);",
        )
        .unwrap();

        let review_data = json!({
            "tasks": [
                {
                    "description": "Create users table with id, email, hash columns",
                    "priority": 1,
                    "spec_section": "1.1",
                    "depends_on": [],
                    "acceptance_criteria": ["Database contains users table with id, email, and password hash columns"],
                    "requires_browser_verification": false,
                    "size": "S"
                },
                {
                    "description": "Add bcrypt hashing to User model",
                    "priority": 2,
                    "spec_section": "1.2",
                    "depends_on": [0],
                    "acceptance_criteria": ["New users persist password_hash instead of plain text", "Automated auth tests cover password hashing"],
                    "requires_browser_verification": true,
                    "size": "M"
                },
            ],
            "removed": [{"original": "old task", "reason": "too vague"}],
            "added": [{"description": "Add bcrypt hashing to User model", "reason": "security"}],
            "splits": [],
            "rewrites": [],
            "merges": []
        });

        let (kept, added, removed) = apply_task_review(&conn, &review_data).unwrap();

        assert_eq!(removed, 1);
        assert_eq!(added, 1);
        assert_eq!(kept, 1);

        // Verify tasks were inserted
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);

        // Verify dependency was created
        let dep_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM task_dependencies", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(dep_count, 1);

        let (criteria_json, requires_browser): (Option<String>, i64) = conn
            .query_row(
                "SELECT acceptance_criteria_json, requires_browser_verification
                 FROM tasks WHERE description = 'Add bcrypt hashing to User model'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let criteria: Vec<String> = serde_json::from_str(&criteria_json.unwrap()).unwrap();
        assert_eq!(criteria.len(), 2);
        assert_eq!(
            criteria[0],
            "New users persist password_hash instead of plain text"
        );
        assert_eq!(requires_browser, 1);
    }

    #[test]
    fn test_parse_test_strategy_response_reads_structured_fields() {
        let data = json!({
            "test_tasks": [
                {
                    "description": "Add compact mode regression coverage",
                    "covers_features": [0, 2],
                    "target_files": ["test/app.test.js"],
                    "rationale": "Single regression file can validate both compact mode slices."
                }
            ]
        });

        let tasks = parse_test_strategy_response(&data);

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].covers_features, vec![0, 2]);
        assert_eq!(tasks[0].target_files, vec!["test/app.test.js"]);
        assert_eq!(tasks[0].depends_on_feature, 0);
    }

    #[test]
    fn test_apply_build_test_config_creates_multi_feature_dependencies() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (
                id INTEGER PRIMARY KEY,
                description TEXT NOT NULL,
                status TEXT DEFAULT 'pending',
                priority INTEGER DEFAULT 5,
                blocked_by TEXT,
                spec_section_id INTEGER,
                prd_section_id TEXT,
                acceptance_criteria_json TEXT,
                requires_browser_verification INTEGER DEFAULT 0,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                started_at TEXT,
                completed_at TEXT,
                total_attempts INTEGER DEFAULT 0,
                total_failures INTEGER DEFAULT 0,
                last_failure_at TEXT
            );
            CREATE TABLE task_dependencies (
                task_id INTEGER NOT NULL,
                depends_on_id INTEGER NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (task_id, depends_on_id)
            );
            CREATE TABLE validation_steps (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                sort_order INTEGER NOT NULL,
                required INTEGER NOT NULL DEFAULT 1,
                timeout_secs INTEGER
            );
            CREATE TABLE config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT DEFAULT CURRENT_TIMESTAMP
            );",
        )
        .unwrap();

        let feature_tasks = vec![
            (
                1,
                "Implement compact mode settings".to_string(),
                1,
                Some("3".to_string()),
            ),
            (
                2,
                "Update compact mode preview".to_string(),
                2,
                Some("3".to_string()),
            ),
        ];

        let config_data = json!({
            "build_cmd": "npm run build",
            "test_cmd": "npm test",
            "test_framework": "node --test",
            "pipeline_steps": [],
            "test_tasks": [
                {
                    "description": "Add regression coverage in `test/app.test.js` for compact mode settings and preview updates",
                    "covers_features": [0, 1],
                    "target_files": ["test/app.test.js"],
                    "rationale": "One regression file can validate both compact mode slices."
                }
            ],
            "build_timeout": 600,
            "test_timeout": 600,
            "rationale": "Use the repo scripts directly."
        });

        let phase6_task_plans = std::collections::HashMap::from([
            (
                normalize_task_text_for_dedupe("Implement compact mode settings"),
                Phase6TaskPlanRecord {
                    task_kind: "feature".to_string(),
                    feature_group: Some("compact-mode-settings".to_string()),
                    coverage_mode: "inline".to_string(),
                },
            ),
            (
                normalize_task_text_for_dedupe("Update compact mode preview"),
                Phase6TaskPlanRecord {
                    task_kind: "feature".to_string(),
                    feature_group: Some("compact-mode-preview".to_string()),
                    coverage_mode: "inline".to_string(),
                },
            ),
        ]);

        let (_build, _test, _steps, test_tasks_count) = apply_build_test_config_with_phase6_plan(
            &conn,
            &config_data,
            &feature_tasks,
            &phase6_task_plans,
        )
        .unwrap();

        assert_eq!(test_tasks_count, 1);
        let inserted_task_id: i64 = conn
            .query_row(
                "SELECT id FROM tasks WHERE description LIKE 'Add regression coverage%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let dep_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM task_dependencies WHERE task_id = ?1",
                [inserted_task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dep_count, 2);
    }

    #[test]
    fn test_apply_build_test_config_skips_overlapping_dedicated_group() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (
                id INTEGER PRIMARY KEY,
                description TEXT NOT NULL,
                status TEXT DEFAULT 'pending',
                priority INTEGER DEFAULT 5,
                blocked_by TEXT,
                spec_section_id INTEGER,
                prd_section_id TEXT,
                acceptance_criteria_json TEXT,
                requires_browser_verification INTEGER DEFAULT 0,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                started_at TEXT,
                completed_at TEXT,
                total_attempts INTEGER DEFAULT 0,
                total_failures INTEGER DEFAULT 0,
                last_failure_at TEXT
            );
            CREATE TABLE task_dependencies (
                task_id INTEGER NOT NULL,
                depends_on_id INTEGER NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (task_id, depends_on_id)
            );
            CREATE TABLE validation_steps (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                command TEXT NOT NULL,
                sort_order INTEGER NOT NULL,
                required INTEGER NOT NULL DEFAULT 1,
                timeout_secs INTEGER
            );
            CREATE TABLE config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT DEFAULT CURRENT_TIMESTAMP
            );",
        )
        .unwrap();

        let feature_tasks = vec![
            (
                1,
                "Implement compact mode settings".to_string(),
                1,
                Some("3".to_string()),
            ),
            (
                2,
                "Update compact mode preview".to_string(),
                2,
                Some("3".to_string()),
            ),
            (
                3,
                "Add automated tests for the settings flow in `test/app.test.js`".to_string(),
                3,
                Some("3".to_string()),
            ),
        ];

        let config_data = json!({
            "build_cmd": "npm run build",
            "test_cmd": "npm test",
            "test_framework": "node --test",
            "pipeline_steps": [],
            "test_tasks": [
                {
                    "description": "Add automated coverage in `test/app.test.js` for settings persistence and preview updates",
                    "covers_features": [0, 1],
                    "target_files": ["test/app.test.js"],
                    "rationale": "Consolidated coverage task."
                }
            ],
            "build_timeout": 600,
            "test_timeout": 600,
            "rationale": "Use the repo scripts directly."
        });

        let phase6_task_plans = std::collections::HashMap::from([
            (
                normalize_task_text_for_dedupe("Implement compact mode settings"),
                Phase6TaskPlanRecord {
                    task_kind: "feature".to_string(),
                    feature_group: Some("compact-mode-settings".to_string()),
                    coverage_mode: "inline".to_string(),
                },
            ),
            (
                normalize_task_text_for_dedupe("Update compact mode preview"),
                Phase6TaskPlanRecord {
                    task_kind: "feature".to_string(),
                    feature_group: Some("compact-mode-preview".to_string()),
                    coverage_mode: "inline".to_string(),
                },
            ),
            (
                normalize_task_text_for_dedupe(
                    "Add automated tests for the settings flow in `test/app.test.js`",
                ),
                Phase6TaskPlanRecord {
                    task_kind: "test".to_string(),
                    feature_group: Some("compact-mode-settings".to_string()),
                    coverage_mode: "dedicated".to_string(),
                },
            ),
        ]);

        let (_build, _test, _steps, test_tasks_count) = apply_build_test_config_with_phase6_plan(
            &conn,
            &config_data,
            &feature_tasks,
            &phase6_task_plans,
        )
        .unwrap();

        assert_eq!(test_tasks_count, 0);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_build_test_config_prompt_warns_about_unicode_dashes() {
        let prompt = build_build_test_config_prompt(&serde_json::json!({}), &[]);

        assert!(prompt.contains("Use only ASCII hyphen-minus characters"));
        assert!(prompt.contains("Prefer single quotes around shell arguments"));
        assert!(prompt.contains("do not emit inline `node -e` or `node --eval` pipeline steps"));
        assert!(!prompt.contains('—'));
        assert!(!prompt.contains('–'));
    }

    #[test]
    fn test_build_test_config_prompt_uses_targeted_context() {
        let prompt = build_build_test_config_prompt(
            &serde_json::json!({
                "vision": {"project_name": "WizardTestProject"},
                "technical": {"platform": {"languages": ["Rust"], "database": "SQLite"}},
                "generate": {"sections": [{"title": "Problem", "content": "content"}]}
            }),
            &[],
        );

        assert!(prompt.contains("Project Summary"));
        assert!(prompt.contains("Technical Details"));
        assert!(prompt.contains("PRD Section Outline"));
        assert!(!prompt.contains("Full PRD Context"));
    }

    #[test]
    fn test_iteration_mode_prompt_uses_summary_not_full_dump() {
        let prompt = build_iteration_mode_prompt(
            &serde_json::json!({
                "vision": {"project_name": "WizardTestProject", "problem_statement": "Needs better wizard progress"},
                "functionality": {"mvp_features": [{"name": "Wizard"}]},
                "technical": {"constraints": ["offline support"], "integrations": []}
            }),
            12,
        );

        assert!(prompt.contains("Project Summary"));
        assert!(prompt.contains("Complexity Indicators"));
        assert!(!prompt.contains("Full PRD Context"));
    }

    // --- JSON Extraction Robustness Tests ---

    #[test]
    fn test_extract_json_brute_simple_object() {
        let input = r#"Here is the JSON: {"key": "value"} and some trailing text"#;
        let result = extract_json_brute(input).unwrap();
        assert_eq!(result, r#"{"key": "value"}"#);
    }

    #[test]
    fn test_extract_json_brute_nested_object() {
        let input = r#"{"outer": {"inner": [1, 2, 3]}, "b": true}"#;
        let result = extract_json_brute(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["outer"]["inner"][1], 2);
    }

    #[test]
    fn test_extract_json_brute_with_strings_containing_braces() {
        let input = r#"Sure! {"msg": "use { and } in strings", "ok": true}"#;
        let result = extract_json_brute(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn test_extract_json_brute_array() {
        let input = r#"Result: [1, 2, {"a": 3}]"#;
        let result = extract_json_brute(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed[2]["a"], 3);
    }

    #[test]
    fn test_extract_json_brute_no_json() {
        let input = "This has no JSON at all.";
        assert!(extract_json_brute(input).is_none());
    }

    #[test]
    fn test_extract_json_brute_escaped_quotes() {
        let input = r#"{"path": "C:\\Users\\test", "ok": true}"#;
        let result = extract_json_brute(input).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn test_normalize_wrapped_json_flattens_string_line_breaks() {
        let input = "{\n  \"msg\": \"line one\n    line two\",\n  \"ok\": true\n}";
        let normalized = normalize_wrapped_json(input);
        let parsed: serde_json::Value = serde_json::from_str(&normalized).unwrap();
        assert_eq!(parsed["msg"], "line one line two");
        assert_eq!(parsed["ok"], true);
    }

    #[test]
    fn test_parse_json_candidate_handles_bulleted_wrapped_json() {
        let input = "● { \"msg\": \"line one\n  line two\", \"ok\": true }";
        let parsed = parse_json_candidate(input).unwrap();
        assert_eq!(parsed["msg"], "line one line two");
        assert_eq!(parsed["ok"], true);
    }
}
