mod cli_handler;

use clap::{Parser, Subcommand};
use dial_core::*;
use dial_providers::AnthropicProvider;
use std::sync::Arc;

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

        /// Skip adding DIAL instructions to AGENTS.md
        #[arg(long)]
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
    Iterate,

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

        /// AI CLI to use (claude, codex, gemini)
        #[arg(long, default_value = "claude")]
        cli: String,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Set a config value
    Set {
        key: String,
        value: String,
    },
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
    if let Commands::Init { phase, import_solutions, no_agents } = command {
        let mut engine = Engine::init(&phase, import_solutions.as_deref(), !no_agents).await?;
        engine.on_event(Arc::new(cli_handler::CliEventHandler));
        return Ok(());
    }

    // New creates a project and runs the full wizard (phases 1-9)
    if let Commands::New { template, from, resume, phase } = command {
        let mut engine = Engine::init(&phase, None, true).await?;
        engine.on_event(Arc::new(cli_handler::CliEventHandler));

        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| errors::DialError::ProviderRequired)?;
        engine.set_provider(Arc::new(AnthropicProvider::new(api_key)));

        engine.new_project(&template, from.as_deref(), resume).await?;
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
            Some(TaskCommands::Add { description, priority, spec, after }) => {
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
            None => {
                engine.task_list(false).await?;
            }
        },

        Commands::Spec { command } => match command {
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
                            println!("{}{} {} ({} words)", indent, s.section_id, s.title, s.word_count);
                        }
                    }
                } else {
                    engine.spec_list().await?;
                }
            }
            Some(SpecCommands::Import { dir }) => {
                engine.prd_import(&dir).await?;
            }
            Some(SpecCommands::Wizard { template, from, resume }) => {
                engine.prd_wizard(&template, from.as_deref(), resume).await?;
            }
            Some(SpecCommands::Migrate) => {
                let count = engine.prd_migrate().await?;
                if count == 0 {
                    println!("{}", output::dim("No spec_sections found to migrate."));
                }
            }
            Some(SpecCommands::Term { command }) => match command {
                TermCommands::Add { canonical, definition, category, variants } => {
                    let variants_json = match variants {
                        Some(v) => {
                            let list: Vec<&str> = v.split(',').map(|s| s.trim()).collect();
                            serde_json::to_string(&list).unwrap_or_else(|_| "[]".to_string())
                        }
                        None => "[]".to_string(),
                    };
                    engine.prd_term_add(&canonical, &variants_json, &definition, &category, None).await?;
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
                    println!("{}", output::dim("No prd.db found. Run 'dial spec import' or 'dial spec wizard'."));
                }
            }
            Some(SpecCommands::Prd { section_id }) => {
                match engine.prd_show(&section_id).await? {
                    Some(section) => {
                        println!("{}", output::bold(&format!("{} {}", section.section_id, section.title)));
                        println!("{}", "=".repeat(60));
                        println!("{}", section.content);
                    }
                    None => {
                        println!("{}", output::dim(&format!("Section '{}' not found.", section_id)));
                    }
                }
            }
            Some(SpecCommands::PrdSearch { query }) => {
                let results = engine.prd_search(&query).await?;
                if results.is_empty() {
                    println!("{}", output::dim("No matching PRD sections."));
                } else {
                    for s in &results {
                        let preview = if s.content.len() > 100 { &s.content[..100] } else { &s.content };
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
                            println!("{}{} {} ({} words)", indent, s.section_id, s.title, s.word_count);
                        }
                    }
                } else {
                    engine.spec_list().await?;
                }
            }
        },

        Commands::Iterate => {
            engine.iterate().await?;
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
                        let notes_str = event.notes.map(|n| format!(" - {}", n)).unwrap_or_default();
                        println!("  {} {}{}{}", event.created_at, event.event_type, conf_str, notes_str);
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

        Commands::Learn { description, category } => {
            engine.learn(&description, category.as_deref()).await?;
        }

        Commands::Learnings { command } => match command {
            Some(LearningsCommands::List { category }) => {
                engine.learnings_list(category.as_deref()).await?;
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

        Commands::Patterns { command } => match command {
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
            Some(PatternCommands::Add { key, description, category, regex }) => {
                engine.patterns_add(&key, &description, &category, &regex, "suggested").await?;
            }
            Some(PatternCommands::Promote { id }) => {
                let new_status = engine.patterns_promote(id).await?;
                println!("Pattern #{} promoted to {}", id, new_status);
            }
            Some(PatternCommands::Suggest) => {
                let suggestions = engine.patterns_suggest().await?;
                if suggestions.is_empty() {
                    println!("{}", output::dim("No pattern suggestions (need 3+ UnknownError occurrences)."));
                } else {
                    println!("{}", output::bold("Suggested Patterns"));
                    println!("{}", "=".repeat(60));
                    for s in suggestions {
                        println!("\n  Common: \"{}\" ({} occurrences)", s.common_substring, s.occurrence_count);
                        for sample in &s.sample_errors {
                            println!("    - {}", output::dim(sample));
                        }
                    }
                    println!("\n{}", output::dim("Use `dial patterns add` to create a pattern from a suggestion."));
                }
            }
        },

        Commands::Pipeline { command } => match command {
            Some(PipelineCommands::Show) | None => {
                let steps = engine.pipeline_list().await?;
                if steps.is_empty() {
                    println!("{}", output::dim("No pipeline steps configured (using build_cmd/test_cmd fallback)."));
                } else {
                    println!("{}", output::bold("Validation Pipeline"));
                    println!("{}", "=".repeat(60));
                    for s in steps {
                        let required_str = if s.required { "required" } else { "optional" };
                        let timeout_str = s.timeout_secs.map(|t| format!(" ({}s timeout)", t)).unwrap_or_default();
                        println!("  #{} [order:{}] {} [{}]{}: {}", s.id, s.sort_order, s.name, required_str, timeout_str, s.command);
                    }
                }
            }
            Some(PipelineCommands::Add { name, command, order, optional, timeout }) => {
                engine.pipeline_add(&name, &command, order, !optional, timeout).await?;
            }
            Some(PipelineCommands::Remove { id }) => {
                engine.pipeline_remove(id).await?;
            }
        },

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
                                println!("{},{},{},{},{:.4},{},{},{:.4}",
                                    t.date, t.iterations, t.successes, t.failures,
                                    t.success_rate, t.tokens_in, t.tokens_out, t.cost_usd);
                            }
                        }
                        _ => {
                            println!("{}", output::bold("Daily Trends"));
                            println!("{}", "=".repeat(60));
                            for t in &trends {
                                println!("{}: {} iters ({} ok, {} fail) {:.0}% | tokens: {}/{} | ${:.4}",
                                    t.date, t.iterations, t.successes, t.failures,
                                    t.success_rate * 100.0, t.tokens_in, t.tokens_out, t.cost_usd);
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
                        println!("Tasks:      {} total, {} completed, {} pending",
                            report.total_tasks, report.completed_tasks, report.pending_tasks);
                        println!("Iterations: {} total, {} completed, {} failed",
                            report.total_iterations, report.completed_iterations, report.failed_iterations);
                        println!("Success:    {:.1}%", report.success_rate * 100.0);
                        println!("Duration:   {:.1}s total, {:.1}s avg/iteration",
                            report.total_duration_secs, report.avg_iteration_duration_secs);
                        println!("Tokens:     {} in, {} out",
                            report.total_tokens_in, report.total_tokens_out);
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
                println!("{}", output::green(&format!("Recovered {} dangling iteration(s).", count)));
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

        Commands::AutoRun { max, cli } => {
            engine.auto_run(max, Some(&cli)).await?;
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

    println!("{}", output::bold(&format!("DIAL Status: {} (phase: {})", project, phase)));
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
            println!("{}", output::yellow(&format!("\nIn Progress: Task #{}", task_id)));
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
    println!("  Completed: {}", task_counts.get("completed").unwrap_or(&0));
    println!("  Blocked:   {}", task_counts.get("blocked").unwrap_or(&0));

    let mut stmt = conn.prepare(
        "SELECT i.id, i.status, i.duration_seconds, t.description
         FROM iterations i
         INNER JOIN tasks t ON i.task_id = t.id
         ORDER BY i.id DESC LIMIT 5",
    )?;

    let recent: Vec<(i64, String, Option<f64>, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
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

            println!("  #{} {:12} {:8} {}", id, status_color, duration_str, desc_preview);
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
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
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

        println!("#{:4} {:12} {:8} {} {}", id, status_color, duration_str, commit, desc_preview);
    }

    Ok(())
}

