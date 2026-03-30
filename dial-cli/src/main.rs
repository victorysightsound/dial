mod cli_handler;
#[cfg(test)]
mod test_support;
mod wizard_backend;

use clap::{Parser, Subcommand, ValueEnum};
use dial_core::*;
use std::path::PathBuf;
use std::sync::Arc;
use wizard_backend::resolve_wizard_provider;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWizardSourceDoc {
    display_path: String,
    prompt_content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CliAgentsMode {
    Local,
    Shared,
    Off,
}

impl From<CliAgentsMode> for AgentsMode {
    fn from(value: CliAgentsMode) -> Self {
        match value {
            CliAgentsMode::Local => AgentsMode::Local,
            CliAgentsMode::Shared => AgentsMode::Shared,
            CliAgentsMode::Off => AgentsMode::Off,
        }
    }
}

fn resolve_wizard_source_doc(from: Option<&str>) -> Result<Option<ResolvedWizardSourceDoc>> {
    let Some(from_path) = from else {
        return Ok(None);
    };

    let prompt_content = dial_core::prd::wizard::load_existing_doc(from_path)?;
    let display_path = std::fs::canonicalize(from_path)
        .unwrap_or_else(|_| PathBuf::from(from_path))
        .to_string_lossy()
        .to_string();

    Ok(Some(ResolvedWizardSourceDoc {
        display_path,
        prompt_content,
    }))
}

fn resolve_agents_mode(agents: Option<CliAgentsMode>, no_agents: bool) -> AgentsMode {
    if no_agents {
        AgentsMode::Off
    } else {
        agents.unwrap_or(CliAgentsMode::Local).into()
    }
}

#[derive(Parser)]
#[command(name = "dial")]
#[command(author = "John Deaton")]
#[command(version = VERSION)]
#[command(about = "DIAL - Deterministic Iterative Agent Loop")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize DIAL in current directory
    Init {
        /// Phase name
        #[arg(long, default_value = DEFAULT_PHASE)]
        phase: String,

        /// Import trusted solutions from another phase
        #[arg(long)]
        import_solutions: Option<String>,

        /// How DIAL should handle AGENTS.md and related agent instruction files
        #[arg(long, value_enum)]
        agents: Option<CliAgentsMode>,

        /// Legacy alias for `--agents off`
        #[arg(long, hide = true)]
        no_agents: bool,
    },

    /// Create a new DIAL project with guided wizard (init + full spec + tasks + config)
    New {
        /// Template to use (spec, architecture, api, mvp)
        #[arg(long, default_value = "spec")]
        template: String,

        /// Existing document to refine
        #[arg(long)]
        from: Option<String>,

        /// Resume a paused wizard session
        #[arg(long)]
        resume: bool,

        /// Phase name
        #[arg(long, default_value = DEFAULT_PHASE)]
        phase: String,

        /// Wizard backend (codex, claude, copilot, gemini, openai-compatible)
        #[arg(long = "wizard-backend", alias = "backend")]
        wizard_backend: Option<String>,

        /// Optional model override for the selected wizard backend
        #[arg(long = "wizard-model", alias = "model")]
        wizard_model: Option<String>,

        /// How DIAL should handle AGENTS.md and related agent instruction files
        #[arg(long, value_enum)]
        agents: Option<CliAgentsMode>,
    },

    /// Index spec files (deprecated: use 'dial spec import' instead)
    Index {
        /// Specs directory
        #[arg(long, default_value = "specs")]
        dir: String,
    },

    /// Manage configuration
    Config {
        #[command(subcommand)]
        command: Option<ConfigCommands>,
    },

    /// Manage tasks
    Task {
        #[command(subcommand)]
        command: Option<TaskCommands>,
    },

    /// Query specs
    Spec {
        #[command(subcommand)]
        command: Option<SpecCommands>,
    },

    /// Run one iteration
    Iterate {
        /// Preview what would happen without creating records or spawning subagents
        #[arg(long)]
        dry_run: bool,

        /// Output format for dry-run (text, json)
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Validate current iteration
    Validate,

    /// Run iterations continuously
    Run {
        /// Max iterations
        #[arg(long)]
        max: Option<u32>,
    },

    /// Stop after current iteration
    Stop,

    /// Show current status
    Status,

    /// Show iteration history
    History {
        /// Number of entries
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,
    },

    /// Show failures
    Failures {
        /// Show all failures
        #[arg(short, long)]
        all: bool,
    },

    /// Show solutions
    Solutions {
        #[command(subcommand)]
        command: Option<SolutionCommands>,
    },

    /// Add a learning
    Learn {
        /// Learning description
        description: String,

        /// Category (build, test, setup, gotcha, pattern, tool, other)
        #[arg(short, long)]
        category: Option<String>,
    },

    /// Show learnings
    Learnings {
        #[command(subcommand)]
        command: Option<LearningsCommands>,
    },

    /// Manage failure patterns
    Patterns {
        #[command(subcommand)]
        command: Option<PatternCommands>,
    },

    /// Manage validation pipeline
    Pipeline {
        #[command(subcommand)]
        command: Option<PipelineCommands>,
    },

    /// Show project health score
    Health {
        /// Output format (text, json)
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Show statistics
    Stats {
        /// Output format (text, json, csv)
        #[arg(long, default_value = "text")]
        format: String,

        /// Show daily trends over the last N days
        #[arg(long)]
        trend: Option<i64>,
    },

    /// Approve a paused iteration (in review/manual mode)
    Approve,

    /// Reject a paused iteration
    Reject {
        /// Reason for rejection
        reason: String,
    },

    /// Migrate data from a v2 DIAL database
    MigrateV2 {
        /// Path to the v2 database file
        path: String,
    },

    /// Recover from crashed/interrupted iterations
    Recover,

    /// Revert to last good commit
    Revert,

    /// Reset current iteration
    Reset,

    /// Show fresh context for current/next task
    Context,

    /// Generate sub-agent prompt for orchestrator mode
    Orchestrate,

    /// Run automated orchestration with fresh AI subprocesses per task
    AutoRun {
        /// Max iterations before stopping
        #[arg(long)]
        max: Option<u32>,

        /// AI CLI to use (claude, codex, copilot, gemini)
        #[arg(long, default_value = "claude")]
        cli: String,

        /// Preview what would happen without creating records or spawning subagents
        #[arg(long)]
        dry_run: bool,

        /// Output format for dry-run (text, json)
        #[arg(long, default_value = "text")]
        format: String,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Set a config value
    Set { key: String, value: String },
    /// Show all config
    Show,
}

#[derive(Subcommand)]
enum TaskCommands {
    /// Add a task
    Add {
        description: String,

        /// Priority (1-10)
        #[arg(short, long, default_value = "5")]
        priority: i32,

        /// Spec section ID
        #[arg(long)]
        spec: Option<i64>,

        /// Task ID this new task depends on (can be repeated)
        #[arg(long = "after")]
        after: Vec<i64>,
    },
    /// List tasks
    List {
        /// Show all tasks
        #[arg(short, long)]
        all: bool,
    },
    /// Show next task
    Next,
    /// Mark task done
    Done { id: i64 },
    /// Block a task
    Block { id: i64, reason: String },
    /// Cancel a task
    Cancel { id: i64 },
    /// Search tasks
    Search { query: String },
    /// Add a dependency (task depends on another)
    Depend {
        /// Task ID
        id: i64,
        /// Task ID it depends on
        on: i64,
    },
    /// Remove a dependency
    Undepend {
        /// Task ID
        id: i64,
        /// Task ID to remove dependency on
        on: i64,
    },
    /// Show dependency info for a task
    Deps { id: i64 },
    /// Show tasks with chronic failures (total_failures >= threshold)
    Chronic {
        /// Minimum total failures to report
        #[arg(long, default_value = "10")]
        threshold: i64,
    },
}

#[derive(Subcommand)]
enum SpecCommands {
    /// Search specs (legacy spec_sections or PRD)
    Search { query: String },
    /// Show spec section (legacy)
    Show { id: i64 },
    /// List spec sections (legacy or PRD)
    List,
    /// Import markdown files into prd.db
    Import {
        /// Directory containing markdown spec files
        #[arg(long, default_value = "specs")]
        dir: String,
    },
    /// Run the PRD wizard to generate a structured spec
    Wizard {
        /// Template to use (spec, architecture, api, mvp)
        #[arg(long, default_value = "spec")]
        template: String,
        /// Existing document to refine
        #[arg(long)]
        from: Option<String>,
        /// Resume a paused wizard session
        #[arg(long)]
        resume: bool,
        /// Wizard backend (codex, claude, copilot, gemini, openai-compatible)
        #[arg(long = "wizard-backend", alias = "backend")]
        wizard_backend: Option<String>,
        /// Optional model override for the selected wizard backend
        #[arg(long = "wizard-model", alias = "model")]
        wizard_model: Option<String>,
    },
    /// Migrate existing spec_sections into prd.db
    Migrate,
    /// Manage terminology
    Term {
        #[command(subcommand)]
        command: TermCommands,
    },
    /// Check PRD status and summary
    Check,
    /// Show a PRD section by dotted ID (e.g., "1.2.3")
    Prd {
        /// Section ID (e.g., "1", "1.2", "1.2.3")
        section_id: String,
    },
    /// Search PRD sections by query
    PrdSearch {
        /// Search query
        query: String,
    },
}

#[derive(Subcommand)]
enum TermCommands {
    /// Add a terminology entry
    Add {
        /// Canonical term name
        canonical: String,
        /// Definition
        definition: String,
        /// Category (e.g., domain, technical, acronym)
        #[arg(short, long, default_value = "domain")]
        category: String,
        /// Comma-separated alternate names/variants
        #[arg(long)]
        variants: Option<String>,
    },
    /// List terminology entries
    List {
        /// Filter by category
        #[arg(short, long)]
        category: Option<String>,
    },
    /// Search terminology
    Search {
        /// Search query
        query: String,
    },
}

#[derive(Subcommand)]
enum SolutionCommands {
    /// List solutions (default: all)
    List {
        /// Show only trusted solutions
        #[arg(short, long)]
        trusted: bool,
    },
    /// Refresh/re-validate a solution (resets decay clock)
    Refresh { id: i64 },
    /// Show history for a solution
    History { id: i64 },
    /// Apply confidence decay to stale solutions
    Decay,
}

#[derive(Subcommand)]
enum PatternCommands {
    /// List all failure patterns
    List,
    /// Add a new pattern
    Add {
        /// Pattern key (unique identifier)
        key: String,
        /// Description
        description: String,
        /// Category (import, syntax, runtime, test, build)
        #[arg(short, long)]
        category: String,
        /// Regex pattern for matching
        #[arg(short, long)]
        regex: String,
    },
    /// Promote a pattern (suggested -> confirmed -> trusted)
    Promote { id: i64 },
    /// Suggest new patterns from unknown error clustering
    Suggest,
    /// Show aggregated metrics per failure pattern
    Metrics {
        /// Output format (text, json)
        #[arg(long, default_value = "text")]
        format: String,
        /// Sort by field (occurrences, cost, time)
        #[arg(long, default_value = "occurrences")]
        sort: String,
    },
}

#[derive(Subcommand)]
enum PipelineCommands {
    /// Show configured pipeline steps
    Show,
    /// Add a pipeline step
    Add {
        /// Step name
        name: String,
        /// Command to run
        command: String,
        /// Sort order (lower runs first)
        #[arg(short, long, default_value = "0")]
        order: i32,
        /// Whether this step is optional (default: required)
        #[arg(long)]
        optional: bool,
        /// Timeout in seconds
        #[arg(short, long)]
        timeout: Option<u64>,
    },
    /// Remove a pipeline step
    Remove { id: i64 },
}

#[derive(Subcommand)]
enum LearningsCommands {
    /// List learnings
    List {
        /// Filter by category
        #[arg(short, long)]
        category: Option<String>,

        /// Filter by failure pattern ID
        #[arg(short, long)]
        pattern: Option<i64>,
    },
    /// Search learnings
    Search { query: String },
    /// Delete a learning
    Delete { id: i64 },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let result = run_command(cli.command).await;

    if let Err(e) = result {
        output::print_error(&e.to_string());
        std::process::exit(1);
    }
}

async fn run_command(command: Commands) -> Result<()> {
    // Init creates a new engine — handled separately
    if let Commands::Init {
        phase,
        import_solutions,
        agents,
        no_agents,
    } = command
    {
        let agents_mode = resolve_agents_mode(agents, no_agents);
        let mut engine =
            Engine::init_with_agents_mode(&phase, import_solutions.as_deref(), agents_mode).await?;
        engine.on_event(Arc::new(cli_handler::CliEventHandler));
        return Ok(());
    }

    // New creates a project and runs the full wizard (phases 1-9)
    if let Commands::New {
        template,
        from,
        resume,
        phase,
        wizard_backend,
        wizard_model,
        agents,
    } = command
    {
        let resolved = resolve_wizard_provider(wizard_backend.as_deref(), wizard_model.as_deref())?;
        let source_doc = resolve_wizard_source_doc(from.as_deref())?;
        let agents_mode = resolve_agents_mode(agents, false);
        let mut engine = open_or_init_new_engine(&phase, resume, agents_mode).await?;
        engine.on_event(Arc::new(cli_handler::CliEventHandler));
        cli_handler::print_wizard_orientation(
            cli_handler::WizardRunKind::Full,
            resolved.backend.as_str(),
            source_doc.as_ref().map(|doc| doc.display_path.as_str()),
        );
        engine.set_provider(resolved.provider);

        engine
            .new_project(
                &template,
                source_doc.as_ref().map(|doc| doc.prompt_content.as_str()),
                resume,
            )
            .await?;
        return Ok(());
    }

    // All other commands require an initialized project
    let mut engine = Engine::open(EngineConfig::default()).await?;
    engine.on_event(Arc::new(cli_handler::CliEventHandler));

    match command {
        Commands::Init { .. } | Commands::New { .. } => unreachable!(),

        Commands::Index { dir } => {
            println!("{}", output::yellow("Note: 'dial index' is deprecated. Use 'dial spec import --dir <path>' instead."));
            println!("{}", output::dim("'dial spec import' writes to the new prd.db with hierarchical sections and FTS5 search."));
            println!();
            engine.index_specs(&dir).await?;
        }

        Commands::Config { command } => match command {
            Some(ConfigCommands::Set { key, value }) => {
                engine.config_set(&key, &value).await?;
            }
            Some(ConfigCommands::Show) | None => {
                engine.config_show().await?;
            }
        },

        Commands::Task { command } => match command {
            Some(TaskCommands::Add {
                description,
                priority,
                spec,
                after,
            }) => {
                let task_id = engine.task_add(&description, priority, spec).await?;
                for dep_id in after {
                    engine.task_depends(task_id, dep_id).await?;
                }
            }
            Some(TaskCommands::List { all }) => {
                engine.task_list(all).await?;
            }
            Some(TaskCommands::Next) => {
                engine.task_next().await?;
            }
            Some(TaskCommands::Done { id }) => {
                engine.task_done(id).await?;
            }
            Some(TaskCommands::Block { id, reason }) => {
                engine.task_block(id, &reason).await?;
            }
            Some(TaskCommands::Cancel { id }) => {
                engine.task_cancel(id).await?;
            }
            Some(TaskCommands::Search { query }) => {
                engine.task_search(&query).await?;
            }
            Some(TaskCommands::Depend { id, on }) => {
                engine.task_depends(id, on).await?;
            }
            Some(TaskCommands::Undepend { id, on }) => {
                engine.task_undepend(id, on).await?;
            }
            Some(TaskCommands::Deps { id }) => {
                engine.task_show_deps(id).await?;
            }
            Some(TaskCommands::Chronic { threshold }) => {
                let results = engine.chronic_failures(threshold).await?;
                if results.is_empty() {
                    println!(
                        "{}",
                        output::dim(&format!(
                            "No tasks with {} or more total failures.",
                            threshold
                        ))
                    );
                } else {
                    println!("{}", output::bold("Chronic Failures"));
                    println!("{}", "=".repeat(70));
                    for r in &results {
                        let last = r.last_failure_at.as_deref().unwrap_or("never");
                        println!(
                            "  #{:<4} failures: {:<4} attempts: {:<4} last: {}",
                            r.task_id, r.total_failures, r.total_attempts, last
                        );
                        println!("        {}", r.description);
                    }
                }
            }
            None => {
                engine.task_list(false).await?;
            }
        },

        Commands::Spec { command } => {
            match command {
                Some(SpecCommands::Search { query }) => {
                    engine.spec_search(&query).await?;
                }
                Some(SpecCommands::Show { id }) => {
                    engine.spec_show(id).await?;
                }
                Some(SpecCommands::List) => {
                    // If prd.db exists, show PRD sections; otherwise legacy
                    if dial_core::prd::prd_db_exists() {
                        let sections = engine.prd_list().await?;
                        if sections.is_empty() {
                            println!("{}", output::dim("No PRD sections. Run 'dial spec import' or 'dial spec wizard'."));
                        } else {
                            println!("{}", output::bold("PRD Sections"));
                            println!("{}", "=".repeat(60));
                            for s in &sections {
                                let indent = "  ".repeat((s.level - 1) as usize);
                                println!(
                                    "{}{} {} ({} words)",
                                    indent, s.section_id, s.title, s.word_count
                                );
                            }
                        }
                    } else {
                        engine.spec_list().await?;
                    }
                }
                Some(SpecCommands::Import { dir }) => {
                    engine.prd_import(&dir).await?;
                }
                Some(SpecCommands::Wizard {
                    template,
                    from,
                    resume,
                    wizard_backend,
                    wizard_model,
                }) => {
                    let resolved = resolve_wizard_provider(
                        wizard_backend.as_deref(),
                        wizard_model.as_deref(),
                    )?;
                    let source_doc = resolve_wizard_source_doc(from.as_deref())?;
                    cli_handler::print_wizard_orientation(
                        cli_handler::WizardRunKind::PrdOnly,
                        resolved.backend.as_str(),
                        source_doc.as_ref().map(|doc| doc.display_path.as_str()),
                    );
                    engine.set_provider(resolved.provider);
                    engine
                        .prd_wizard(
                            &template,
                            source_doc.as_ref().map(|doc| doc.prompt_content.as_str()),
                            resume,
                        )
                        .await?;
                }
                Some(SpecCommands::Migrate) => {
                    let count = engine.prd_migrate().await?;
                    if count == 0 {
                        println!("{}", output::dim("No spec_sections found to migrate."));
                    }
                }
                Some(SpecCommands::Term { command }) => match command {
                    TermCommands::Add {
                        canonical,
                        definition,
                        category,
                        variants,
                    } => {
                        let variants_json = match variants {
                            Some(v) => {
                                let list: Vec<&str> = v.split(',').map(|s| s.trim()).collect();
                                serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string())
                            }
                            None => "[]".to_string(),
                        };
                        engine
                            .prd_term_add(&canonical, &variants_json, &definition, &category, None)
                            .await?;
                    }
                    TermCommands::List { category } => {
                        let terms = engine.prd_term_list(category.as_deref()).await?;
                        if terms.is_empty() {
                            println!("{}", output::dim("No terminology entries."));
                        } else {
                            println!("{}", output::bold("Terminology"));
                            println!("{}", "=".repeat(60));
                            for t in &terms {
                                println!("  {} [{}]: {}", t.canonical, t.category, t.definition);
                            }
                        }
                    }
                    TermCommands::Search { query } => {
                        let terms = engine.prd_term_search(&query).await?;
                        if terms.is_empty() {
                            println!("{}", output::dim("No matching terms."));
                        } else {
                            for t in &terms {
                                println!("  {} [{}]: {}", t.canonical, t.category, t.definition);
                            }
                        }
                    }
                },
                Some(SpecCommands::Check) => {
                    if dial_core::prd::prd_db_exists() {
                        let sections = engine.prd_list().await?;
                        let terms = engine.prd_term_list(None).await?;
                        let total_words: i32 = sections.iter().map(|s| s.word_count).sum();
                        println!("{}", output::bold("PRD Status"));
                        println!("{}", "=".repeat(40));
                        println!("  Sections:    {}", sections.len());
                        println!("  Word count:  {}", total_words);
                        println!("  Terms:       {}", terms.len());
                        output::print_success("prd.db is healthy.");
                    } else {
                        println!(
                            "{}",
                            output::dim(
                                "No prd.db found. Run 'dial spec import' or 'dial spec wizard'."
                            )
                        );
                    }
                }
                Some(SpecCommands::Prd { section_id }) => {
                    match engine.prd_show(&section_id).await? {
                        Some(section) => {
                            println!(
                                "{}",
                                output::bold(&format!("{} {}", section.section_id, section.title))
                            );
                            println!("{}", "=".repeat(60));
                            println!("{}", section.content);
                        }
                        None => {
                            println!(
                                "{}",
                                output::dim(&format!("Section '{}' not found.", section_id))
                            );
                        }
                    }
                }
                Some(SpecCommands::PrdSearch { query }) => {
                    let results = engine.prd_search(&query).await?;
                    if results.is_empty() {
                        println!("{}", output::dim("No matching PRD sections."));
                    } else {
                        for s in &results {
                            let preview = if s.content.len() > 100 {
                                &s.content[..100]
                            } else {
                                &s.content
                            };
                            println!("  {} {} - {}", s.section_id, s.title, preview);
                        }
                    }
                }
                None => {
                    // Default: show PRD sections if available, else legacy
                    if dial_core::prd::prd_db_exists() {
                        let sections = engine.prd_list().await?;
                        if sections.is_empty() {
                            println!("{}", output::dim("No PRD sections. Run 'dial spec import' or 'dial spec wizard'."));
                        } else {
                            println!("{}", output::bold("PRD Sections"));
                            println!("{}", "=".repeat(60));
                            for s in &sections {
                                let indent = "  ".repeat((s.level - 1) as usize);
                                println!(
                                    "{}{} {} ({} words)",
                                    indent, s.section_id, s.title, s.word_count
                                );
                            }
                        }
                    } else {
                        engine.spec_list().await?;
                    }
                }
            }
        }

        Commands::Iterate { dry_run, format } => {
            if dry_run {
                let result = engine.iterate_dry_run().await?;
                if format == "json" {
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                } else {
                    print_dry_run_result(&result);
                }
            } else {
                engine.iterate().await?;
            }
        }

        Commands::Validate => {
            engine.validate().await?;
        }

        Commands::Run { max } => {
            engine.run(max).await?;
        }

        Commands::Stop => {
            engine.stop().await?;
        }

        Commands::Status => {
            show_status()?;
        }

        Commands::History { limit } => {
            show_history(limit)?;
        }

        Commands::Failures { all } => {
            engine.show_failures(!all).await?;
        }

        Commands::Solutions { command } => match command {
            Some(SolutionCommands::List { trusted }) => {
                engine.show_solutions(trusted).await?;
            }
            None => {
                engine.show_solutions(false).await?;
            }
            Some(SolutionCommands::Refresh { id }) => {
                engine.solutions_refresh(id).await?;
            }
            Some(SolutionCommands::History { id }) => {
                let events = engine.solutions_history(id).await?;
                if events.is_empty() {
                    println!("{}", output::dim("No history for this solution."));
                } else {
                    println!("{}", output::bold(&format!("Solution #{} History", id)));
                    println!("{}", "=".repeat(60));
                    for event in events {
                        let conf_str = match (event.old_confidence, event.new_confidence) {
                            (Some(old), Some(new)) => format!(" ({:.2} -> {:.2})", old, new),
                            _ => String::new(),
                        };
                        let notes_str =
                            event.notes.map(|n| format!(" - {}", n)).unwrap_or_default();
                        println!(
                            "  {} {}{}{}",
                            event.created_at, event.event_type, conf_str, notes_str
                        );
                    }
                }
            }
            Some(SolutionCommands::Decay) => {
                let count = engine.solutions_decay().await?;
                if count == 0 {
                    println!("{}", output::dim("No solutions needed decay."));
                }
            }
        },

        Commands::Learn {
            description,
            category,
        } => {
            engine.learn(&description, category.as_deref()).await?;
        }

        Commands::Learnings { command } => match command {
            Some(LearningsCommands::List { category, pattern }) => {
                if let Some(pid) = pattern {
                    engine.learnings_list_for_pattern(pid).await?;
                } else {
                    engine.learnings_list(category.as_deref()).await?;
                }
            }
            Some(LearningsCommands::Search { query }) => {
                engine.learnings_search(&query).await?;
            }
            Some(LearningsCommands::Delete { id }) => {
                engine.learnings_delete(id).await?;
            }
            None => {
                engine.learnings_list(None).await?;
            }
        },

        Commands::Patterns { command } => {
            match command {
                Some(PatternCommands::List) | None => {
                    let patterns = engine.patterns_list().await?;
                    if patterns.is_empty() {
                        println!("{}", output::dim("No patterns configured."));
                    } else {
                        println!("{}", output::bold("Failure Patterns"));
                        println!("{}", "=".repeat(80));
                        for p in patterns {
                            let regex_str = p.regex_pattern.as_deref().unwrap_or("(no regex)");
                            let cat = p.category.as_deref().unwrap_or("unknown");
                            println!(
                                "  #{:<4} [{}] {:20} {:15} {} ({}x)",
                                p.id, p.status, p.pattern_key, cat, regex_str, p.occurrence_count
                            );
                        }
                    }
                }
                Some(PatternCommands::Add {
                    key,
                    description,
                    category,
                    regex,
                }) => {
                    engine
                        .patterns_add(&key, &description, &category, &regex, "suggested")
                        .await?;
                }
                Some(PatternCommands::Promote { id }) => {
                    let new_status = engine.patterns_promote(id).await?;
                    println!("Pattern #{} promoted to {}", id, new_status);
                }
                Some(PatternCommands::Suggest) => {
                    let suggestions = engine.patterns_suggest().await?;
                    if suggestions.is_empty() {
                        println!(
                            "{}",
                            output::dim(
                                "No pattern suggestions (need 3+ UnknownError occurrences)."
                            )
                        );
                    } else {
                        println!("{}", output::bold("Suggested Patterns"));
                        println!("{}", "=".repeat(60));
                        for s in suggestions {
                            println!(
                                "\n  Common: \"{}\" ({} occurrences)",
                                s.common_substring, s.occurrence_count
                            );
                            for sample in &s.sample_errors {
                                println!("    - {}", output::dim(sample));
                            }
                        }
                        println!(
                            "\n{}",
                            output::dim(
                                "Use `dial patterns add` to create a pattern from a suggestion."
                            )
                        );
                    }
                }
                Some(PatternCommands::Metrics { format, sort }) => {
                    let mut metrics = engine.pattern_metrics().await?;
                    if metrics.is_empty() {
                        println!(
                            "{}",
                            output::dim("No pattern metrics (no failures recorded).")
                        );
                    } else {
                        match sort.as_str() {
                            "cost" => metrics.sort_by(|a, b| {
                                b.total_cost_usd
                                    .partial_cmp(&a.total_cost_usd)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            }),
                            "time" => metrics.sort_by(|a, b| {
                                b.total_resolution_time_secs
                                    .partial_cmp(&a.total_resolution_time_secs)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            }),
                            _ => metrics
                                .sort_by(|a, b| b.total_occurrences.cmp(&a.total_occurrences)),
                        }

                        if format == "json" {
                            let items: Vec<String> = metrics.iter().map(|m| m.to_json()).collect();
                            println!("[{}]", items.join(","));
                        } else {
                            println!("{}", output::bold("Pattern Metrics"));
                            println!("{}", "=".repeat(100));
                            println!(
                                "  {:<25} {:<10} {:>6} {:>10} {:>10} {:>12} {:>6} {:>6} {:>6}",
                                "Pattern",
                                "Category",
                                "Count",
                                "Avg Time",
                                "Cost",
                                "Tokens",
                                "Auto",
                                "Manual",
                                "Open"
                            );
                            println!("  {}", "-".repeat(96));
                            for m in &metrics {
                                println!(
                                "  {:<25} {:<10} {:>6} {:>9.1}s ${:>9.4} {:>12} {:>6} {:>6} {:>6}",
                                m.pattern_key, m.category, m.total_occurrences,
                                m.avg_resolution_time_secs, m.total_cost_usd,
                                m.total_tokens_consumed,
                                m.auto_resolved_count, m.manual_resolved_count, m.unresolved_count
                            );
                            }
                        }
                    }
                }
            }
        }

        Commands::Pipeline { command } => match command {
            Some(PipelineCommands::Show) | None => {
                let steps = engine.pipeline_list().await?;
                if steps.is_empty() {
                    println!(
                        "{}",
                        output::dim(
                            "No pipeline steps configured (using build_cmd/test_cmd fallback)."
                        )
                    );
                } else {
                    println!("{}", output::bold("Validation Pipeline"));
                    println!("{}", "=".repeat(60));
                    for s in steps {
                        let required_str = if s.required { "required" } else { "optional" };
                        let timeout_str = s
                            .timeout_secs
                            .map(|t| format!(" ({}s timeout)", t))
                            .unwrap_or_default();
                        println!(
                            "  #{} [order:{}] {} [{}]{}: {}",
                            s.id, s.sort_order, s.name, required_str, timeout_str, s.command
                        );
                    }
                }
            }
            Some(PipelineCommands::Add {
                name,
                command,
                order,
                optional,
                timeout,
            }) => {
                engine
                    .pipeline_add(&name, &command, order, !optional, timeout)
                    .await?;
            }
            Some(PipelineCommands::Remove { id }) => {
                engine.pipeline_remove(id).await?;
            }
        },

        Commands::Health { format } => {
            let health = engine.health().await?;
            if format == "json" {
                println!("{}", serde_json::to_string_pretty(&health).unwrap());
            } else {
                print_health(&health);
            }
        }

        Commands::Stats { format, trend } => {
            if let Some(days) = trend {
                let trends = engine.trends(days).await?;
                if trends.is_empty() {
                    println!("No data in the last {} days.", days);
                } else {
                    match format.as_str() {
                        "json" => {
                            let items: Vec<String> = trends.iter().map(|t| t.to_json()).collect();
                            println!("[{}]", items.join(","));
                        }
                        "csv" => {
                            println!("date,iterations,successes,failures,success_rate,tokens_in,tokens_out,cost_usd");
                            for t in &trends {
                                println!(
                                    "{},{},{},{},{:.4},{},{},{:.4}",
                                    t.date,
                                    t.iterations,
                                    t.successes,
                                    t.failures,
                                    t.success_rate,
                                    t.tokens_in,
                                    t.tokens_out,
                                    t.cost_usd
                                );
                            }
                        }
                        _ => {
                            println!("{}", output::bold("Daily Trends"));
                            println!("{}", "=".repeat(60));
                            for t in &trends {
                                println!(
                                    "{}: {} iters ({} ok, {} fail) {:.0}% | tokens: {}/{} | ${:.4}",
                                    t.date,
                                    t.iterations,
                                    t.successes,
                                    t.failures,
                                    t.success_rate * 100.0,
                                    t.tokens_in,
                                    t.tokens_out,
                                    t.cost_usd
                                );
                            }
                        }
                    }
                }
            } else {
                let report = engine.stats().await?;
                match format.as_str() {
                    "json" => println!("{}", report.to_json()),
                    "csv" => println!("{}", report.to_csv()),
                    _ => {
                        println!("{}", output::bold("DIAL Statistics"));
                        println!("{}", "=".repeat(60));
                        println!(
                            "Tasks:      {} total, {} completed, {} pending",
                            report.total_tasks, report.completed_tasks, report.pending_tasks
                        );
                        println!(
                            "Iterations: {} total, {} completed, {} failed",
                            report.total_iterations,
                            report.completed_iterations,
                            report.failed_iterations
                        );
                        println!("Success:    {:.1}%", report.success_rate * 100.0);
                        println!(
                            "Duration:   {:.1}s total, {:.1}s avg/iteration",
                            report.total_duration_secs, report.avg_iteration_duration_secs
                        );
                        println!(
                            "Tokens:     {} in, {} out",
                            report.total_tokens_in, report.total_tokens_out
                        );
                        println!("Cost:       ${:.4}", report.total_cost_usd);
                        println!("Failures:   {}", report.total_failures);
                        println!("Learnings:  {}", report.total_learnings);
                    }
                }
            }
        }

        Commands::Recover => {
            let count = engine.recover().await?;
            if count > 0 {
                println!(
                    "{}",
                    output::green(&format!("Recovered {} dangling iteration(s).", count))
                );
            } else {
                println!("No dangling iterations found.");
            }
        }

        Commands::MigrateV2 { path } => {
            engine.migrate_v2(&path).await?;
        }

        Commands::Approve => {
            engine.approve().await?;
        }

        Commands::Reject { reason } => {
            engine.reject(&reason).await?;
        }

        Commands::Revert => {
            engine.revert().await?;
        }

        Commands::Reset => {
            engine.reset().await?;
        }

        Commands::Context => {
            engine.show_context().await?;
        }

        Commands::Orchestrate => {
            engine.orchestrate().await?;
        }

        Commands::AutoRun {
            max,
            cli,
            dry_run,
            format,
        } => {
            if dry_run {
                let result = engine.iterate_dry_run().await?;
                if format == "json" {
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                } else {
                    print_dry_run_result(&result);
                }
            } else {
                engine.auto_run(max, Some(&cli)).await?;
            }
        }
    }

    Ok(())
}

// --- CLI-specific display functions ---
// These access the DB directly for presentation. They'll be refactored
// to use structured Engine returns when the event system is added (Phase 2).

fn show_status() -> Result<()> {
    let conn = get_db(None)?;
    let phase = get_current_phase()?;
    let project = config::config_get("project_name")?.unwrap_or_else(|| "unknown".to_string());

    println!(
        "{}",
        output::bold(&format!("DIAL Status: {} (phase: {})", project, phase))
    );
    println!("{}", "=".repeat(60));

    let current: Option<(i64, i64, String, i32)> = conn
        .query_row(
            "SELECT i.id, i.task_id, t.description, i.attempt_number
             FROM iterations i
             INNER JOIN tasks t ON i.task_id = t.id
             WHERE i.status = 'in_progress'
             ORDER BY i.id DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .ok();

    match current {
        Some((_, task_id, description, attempt)) => {
            println!(
                "{}",
                output::yellow(&format!("\nIn Progress: Task #{}", task_id))
            );
            println!("  {}", description);
            println!("  Attempt {} of {}", attempt, MAX_FIX_ATTEMPTS);
        }
        None => {
            println!("{}", output::dim("\nNo iteration in progress."));
        }
    }

    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM tasks GROUP BY status")?;
    let task_counts: std::collections::HashMap<String, i64> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    println!("\nTasks:");
    println!("  Pending:   {}", task_counts.get("pending").unwrap_or(&0));
    println!(
        "  Completed: {}",
        task_counts.get("completed").unwrap_or(&0)
    );
    println!("  Blocked:   {}", task_counts.get("blocked").unwrap_or(&0));

    let mut stmt = conn.prepare(
        "SELECT i.id, i.status, i.duration_seconds, t.description
         FROM iterations i
         INNER JOIN tasks t ON i.task_id = t.id
         ORDER BY i.id DESC LIMIT 5",
    )?;

    let recent: Vec<(i64, String, Option<f64>, String)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if !recent.is_empty() {
        println!("\nRecent Iterations:");
        for (id, status, duration, description) in recent {
            let status_color = if status == "completed" {
                output::green(&status)
            } else {
                output::red(&status)
            };

            let duration_str = duration
                .map(|d| format!("{:.1}s", d))
                .unwrap_or_else(|| "...".to_string());

            let desc_preview = if description.len() > 40 {
                &description[..40]
            } else {
                &description
            };

            println!(
                "  #{} {:12} {:8} {}",
                id, status_color, duration_str, desc_preview
            );
        }
    }

    Ok(())
}

fn show_history(limit: usize) -> Result<()> {
    let conn = get_db(None)?;

    let mut stmt = conn.prepare(
        "SELECT i.id, i.status, i.duration_seconds, i.commit_hash, t.description
         FROM iterations i
         INNER JOIN tasks t ON i.task_id = t.id
         ORDER BY i.id DESC LIMIT ?1",
    )?;

    let rows: Vec<(i64, String, Option<f64>, Option<String>, String)> = stmt
        .query_map([limit as i64], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if rows.is_empty() {
        println!("{}", output::dim("No iteration history."));
        return Ok(());
    }

    println!("{}", output::bold("Iteration History"));
    println!("{}", "=".repeat(80));

    for (id, status, duration, commit_hash, description) in rows {
        let status_color = match status.as_str() {
            "completed" => output::green(&status),
            "failed" => output::red(&status),
            "reverted" => output::yellow(&status),
            "in_progress" => output::blue(&status),
            _ => status.clone(),
        };

        let duration_str = duration
            .map(|d| format!("{:.1}s", d))
            .unwrap_or_else(|| "...".to_string());

        let commit = commit_hash
            .map(|h| h[..8].to_string())
            .unwrap_or_else(|| "--------".to_string());

        let desc_preview = if description.len() > 40 {
            &description[..40]
        } else {
            &description
        };

        println!(
            "#{:4} {:12} {:8} {} {}",
            id, status_color, duration_str, commit, desc_preview
        );
    }

    Ok(())
}

fn print_dry_run_result(result: &DryRunResult) {
    println!("{}", output::bold("DIAL Dry Run Preview"));
    println!("{}", "=".repeat(70));
    println!();

    println!(
        "{}",
        output::bold(&format!(
            "Task #{}: {}",
            result.task.id, result.task.description
        ))
    );
    println!("  Priority:     {}", result.task.priority);
    println!(
        "  Dependencies: {}",
        if result.dependencies_satisfied {
            output::green("satisfied")
        } else {
            output::red("NOT satisfied")
        }
    );
    println!();

    println!("{}", output::bold("Context Budget"));
    println!("  Token budget: {}", result.token_budget);
    println!("  Tokens used:  {}", result.total_context_tokens);
    println!();

    if !result.context_items_included.is_empty() {
        println!("{}", output::bold("Included Context Items"));
        for (label, tokens) in &result.context_items_included {
            println!("  {} ({} tokens)", output::green(label), tokens);
        }
        println!();
    }

    if !result.context_items_excluded.is_empty() {
        println!(
            "{}",
            output::bold("Excluded Context Items (budget exceeded)")
        );
        for (label, tokens) in &result.context_items_excluded {
            println!("  {} ({} tokens)", output::yellow(label), tokens);
        }
        println!();
    }

    if !result.suggested_solutions.is_empty() {
        println!("{}", output::bold("Suggested Solutions"));
        for sol in &result.suggested_solutions {
            println!("  - {}", sol);
        }
        println!();
    }

    println!("{}", output::bold("Prompt Preview (first 500 chars)"));
    println!("{}", output::dim(&"-".repeat(70)));
    println!("{}", result.prompt_preview);
    if result.prompt_preview.len() >= 500 {
        println!("{}", output::dim("..."));
    }
    println!("{}", output::dim(&"-".repeat(70)));
}

async fn open_or_init_new_engine(
    phase: &str,
    resume: bool,
    agents_mode: AgentsMode,
) -> Result<Engine> {
    if resume {
        Engine::open(EngineConfig::default()).await
    } else {
        Engine::init_with_agents_mode(phase, None, agents_mode).await
    }
}

fn print_health(health: &dial_core::health::HealthScore) {
    let score_str = format!("{}", health.score);
    let colored_score = if health.score >= 70 {
        output::green(&score_str)
    } else if health.score >= 40 {
        output::yellow(&score_str)
    } else {
        output::red(&score_str)
    };

    let trend_str = format!("{}", health.trend);
    let colored_trend = match health.trend {
        dial_core::Trend::Improving => output::green(&trend_str),
        dial_core::Trend::Stable => output::yellow(&trend_str),
        dial_core::Trend::Declining => output::red(&trend_str),
    };

    println!("{}", output::bold("Project Health"));
    println!("{}", "=".repeat(60));
    println!("Score: {}/100  Trend: {}", colored_score, colored_trend);
    println!();

    println!("{}", output::bold("Factors"));
    println!("{}", "-".repeat(60));
    for factor in &health.factors {
        let factor_score_str = format!("{:>3}", factor.score);
        let colored_factor = if factor.score >= 70 {
            output::green(&factor_score_str)
        } else if factor.score >= 40 {
            output::yellow(&factor_score_str)
        } else {
            output::red(&factor_score_str)
        };

        println!(
            "  {:<28} {} (weight: {:.2})",
            factor.name, colored_factor, factor.weight
        );
        println!("    {}", output::dim(&factor.detail));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::cwd_lock;
    use std::env;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_open_or_init_new_engine_resume_uses_existing_project() {
        let _guard = cwd_lock().lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = env::current_dir().unwrap();
        let temp = tempdir().unwrap();
        env::set_current_dir(temp.path()).unwrap();

        let _engine = Engine::init("mvp", None, true).await.unwrap();

        let reopened = open_or_init_new_engine("mvp", true, AgentsMode::Local).await;
        assert!(reopened.is_ok(), "resume should reopen an existing project");

        env::set_current_dir(original_dir).unwrap();
    }

    #[test]
    fn resolve_wizard_source_doc_loads_content_and_canonicalizes_display_path() {
        let _guard = cwd_lock().lock().unwrap_or_else(|e| e.into_inner());
        let original_dir = env::current_dir().unwrap();
        let temp = tempdir().unwrap();
        let source = temp.path().join("wizard-source.md");
        fs::write(&source, "# Example\n\nLoaded from disk.\n").unwrap();
        env::set_current_dir(temp.path()).unwrap();

        let resolved = resolve_wizard_source_doc(Some("wizard-source.md"))
            .unwrap()
            .unwrap();

        assert_eq!(resolved.prompt_content, "# Example\n\nLoaded from disk.\n");
        assert_eq!(
            PathBuf::from(&resolved.display_path),
            source.canonicalize().unwrap()
        );

        env::set_current_dir(original_dir).unwrap();
    }

    #[test]
    fn resolve_agents_mode_defaults_to_local() {
        assert_eq!(resolve_agents_mode(None, false), AgentsMode::Local);
    }

    #[test]
    fn resolve_agents_mode_honors_explicit_modes() {
        assert_eq!(
            resolve_agents_mode(Some(CliAgentsMode::Shared), false),
            AgentsMode::Shared
        );
        assert_eq!(
            resolve_agents_mode(Some(CliAgentsMode::Off), false),
            AgentsMode::Off
        );
    }

    #[test]
    fn resolve_agents_mode_legacy_alias_forces_off() {
        assert_eq!(
            resolve_agents_mode(Some(CliAgentsMode::Shared), true),
            AgentsMode::Off
        );
    }
}
