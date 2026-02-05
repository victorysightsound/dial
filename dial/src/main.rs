use clap::{Parser, Subcommand};
use dial::*;

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

    /// Index spec files
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
        /// Show only trusted solutions
        #[arg(short, long)]
        trusted: bool,
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

    /// Show statistics
    Stats,

    /// Revert to last good commit
    Revert,

    /// Reset current iteration
    Reset,

    /// Show fresh context for current/next task (Ralph-style)
    Context,

    /// Generate sub-agent prompt for orchestrator mode (Ralph-style)
    Orchestrate,
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
}

#[derive(Subcommand)]
enum SpecCommands {
    /// Search specs
    Search { query: String },
    /// Show spec section
    Show { id: i64 },
    /// List spec sections
    List,
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

fn main() {
    let cli = Cli::parse();

    let result = run_command(cli.command);

    if let Err(e) = result {
        output::print_error(&e.to_string());
        std::process::exit(1);
    }
}

fn run_command(command: Commands) -> Result<()> {
    // Commands that don't need initialization
    if let Commands::Init { phase, import_solutions, no_agents } = command {
        init_db(&phase, import_solutions.as_deref(), !no_agents)?;
        return Ok(());
    }

    // Check if initialized (for all other commands)
    if !get_dial_dir().exists() {
        return Err(DialError::NotInitialized);
    }

    match command {
        Commands::Init { .. } => unreachable!(),

        Commands::Index { dir } => {
            spec::index_specs(&dir)?;
        }

        Commands::Config { command } => match command {
            Some(ConfigCommands::Set { key, value }) => {
                config::config_set(&key, &value)?;
            }
            Some(ConfigCommands::Show) | None => {
                config::config_show()?;
            }
        },

        Commands::Task { command } => match command {
            Some(TaskCommands::Add { description, priority, spec }) => {
                task::task_add(&description, priority, spec)?;
            }
            Some(TaskCommands::List { all }) => {
                task::task_list(all)?;
            }
            Some(TaskCommands::Next) => {
                task::task_next()?;
            }
            Some(TaskCommands::Done { id }) => {
                task::task_done(id)?;
            }
            Some(TaskCommands::Block { id, reason }) => {
                task::task_block(id, &reason)?;
            }
            Some(TaskCommands::Cancel { id }) => {
                task::task_cancel(id)?;
            }
            Some(TaskCommands::Search { query }) => {
                task::task_search(&query)?;
            }
            None => {
                task::task_list(false)?;
            }
        },

        Commands::Spec { command } => match command {
            Some(SpecCommands::Search { query }) => {
                spec::spec_search(&query)?;
            }
            Some(SpecCommands::Show { id }) => {
                spec::spec_show(id)?;
            }
            Some(SpecCommands::List) | None => {
                spec::spec_list()?;
            }
        },

        Commands::Iterate => {
            iteration::iterate_once()?;
        }

        Commands::Validate => {
            iteration::validate_current()?;
        }

        Commands::Run { max } => {
            iteration::run_loop(max)?;
        }

        Commands::Stop => {
            iteration::stop_loop()?;
        }

        Commands::Status => {
            show_status()?;
        }

        Commands::History { limit } => {
            show_history(limit)?;
        }

        Commands::Failures { all } => {
            failure::show_failures(!all)?;
        }

        Commands::Solutions { trusted } => {
            failure::show_solutions(trusted)?;
        }

        Commands::Learn { description, category } => {
            learning::add_learning(&description, category.as_deref())?;
        }

        Commands::Learnings { command } => match command {
            Some(LearningsCommands::List { category }) => {
                learning::list_learnings(category.as_deref())?;
            }
            Some(LearningsCommands::Search { query }) => {
                learning::search_learnings(&query)?;
            }
            Some(LearningsCommands::Delete { id }) => {
                learning::delete_learning(id)?;
            }
            None => {
                learning::list_learnings(None)?;
            }
        },

        Commands::Stats => {
            show_stats()?;
        }

        Commands::Revert => {
            iteration::revert_to_last_good()?;
        }

        Commands::Reset => {
            iteration::reset_current()?;
        }

        Commands::Context => {
            iteration::show_context()?;
        }

        Commands::Orchestrate => {
            iteration::orchestrate()?;
        }
    }

    Ok(())
}

fn show_status() -> Result<()> {
    let conn = get_db(None)?;
    let phase = get_current_phase()?;
    let project = config::config_get("project_name")?.unwrap_or_else(|| "unknown".to_string());

    println!("{}", output::bold(&format!("DIAL Status: {} (phase: {})", project, phase)));
    println!("{}", "=".repeat(60));

    // Current iteration
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

    // Task counts
    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM tasks GROUP BY status")?;
    let task_counts: std::collections::HashMap<String, i64> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    println!("\nTasks:");
    println!("  Pending:   {}", task_counts.get("pending").unwrap_or(&0));
    println!("  Completed: {}", task_counts.get("completed").unwrap_or(&0));
    println!("  Blocked:   {}", task_counts.get("blocked").unwrap_or(&0));

    // Recent iterations
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

fn show_stats() -> Result<()> {
    let conn = get_db(None)?;
    let phase = get_current_phase()?;
    let project = config::config_get("project_name")?.unwrap_or_else(|| "unknown".to_string());

    println!("{}", output::bold(&format!("\nDIAL Statistics: {} (phase: {})", project, phase)));
    println!("{}", "=".repeat(60));

    // Iterations
    let (total, completed, failed, total_duration, avg_duration, max_duration): (
        i64, i64, i64, Option<f64>, Option<f64>, Option<f64>
    ) = conn.query_row(
        "SELECT
            COUNT(*),
            COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0),
            SUM(duration_seconds),
            AVG(duration_seconds),
            MAX(duration_seconds)
         FROM iterations",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;

    let success_rate = if total > 0 {
        completed as f64 / total as f64 * 100.0
    } else {
        0.0
    };

    println!("\n{}", output::bold("Iterations"));
    println!("  Total:      {}", total);
    println!("  Successful: {} ({:.1}%)", output::green(&completed.to_string()), success_rate);
    if failed > 0 {
        println!("  Failed:     {} ({:.1}%)", output::red(&failed.to_string()), 100.0 - success_rate);
    } else {
        println!("  Failed:     {}", failed);
    }

    // Tasks
    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM tasks GROUP BY status")?;
    let task_counts: std::collections::HashMap<String, i64> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    println!("\n{}", output::bold("Tasks"));
    println!("  Completed:  {}", task_counts.get("completed").unwrap_or(&0));
    println!("  Pending:    {}", task_counts.get("pending").unwrap_or(&0));
    println!("  Blocked:    {}", task_counts.get("blocked").unwrap_or(&0));
    println!("  Cancelled:  {}", task_counts.get("cancelled").unwrap_or(&0));

    // Time
    if let Some(total_dur) = total_duration {
        let total_mins = total_dur / 60.0;
        let avg_mins = avg_duration.unwrap_or(0.0) / 60.0;
        let max_mins = max_duration.unwrap_or(0.0) / 60.0;

        println!("\n{}", output::bold("Time"));
        if total_mins >= 60.0 {
            println!("  Total runtime:    {:.1}h", total_mins / 60.0);
        } else {
            println!("  Total runtime:    {:.1}m", total_mins);
        }
        println!("  Avg iteration:    {:.1}m", avg_mins);
        println!("  Longest:          {:.1}m", max_mins);
    }

    // Failure patterns
    let mut stmt = conn.prepare(
        "SELECT pattern_key, occurrence_count
         FROM failure_patterns
         ORDER BY occurrence_count DESC LIMIT 5",
    )?;

    let patterns: Vec<(String, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();

    if !patterns.is_empty() {
        println!("\n{}", output::bold("Failure Patterns (top 5)"));
        for (pattern_key, count) in patterns {
            println!("  {:25} {} occurrences", pattern_key, count);
        }
    }

    // Solutions
    let (sol_total, sol_trusted, sol_success, sol_failure): (i64, i64, i64, i64) = conn.query_row(
        "SELECT
            COUNT(*),
            COALESCE(SUM(CASE WHEN confidence >= ?1 THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(success_count), 0),
            COALESCE(SUM(failure_count), 0)
         FROM solutions",
        [TRUST_THRESHOLD],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;

    if sol_total > 0 {
        let total_apps = sol_success + sol_failure;
        let hit_rate = if total_apps > 0 {
            sol_success as f64 / total_apps as f64 * 100.0
        } else {
            0.0
        };

        println!("\n{}", output::bold("Solutions"));
        println!("  Total:            {}", sol_total);
        println!("  Trusted (≥0.6):   {}", output::green(&sol_trusted.to_string()));
        if total_apps > 0 {
            println!("  Hit rate:         {:.0}% ({} applications)", hit_rate, total_apps);
        }
    }

    // Learnings
    let (learn_total, learn_refs): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(times_referenced), 0) FROM learnings",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    if learn_total > 0 {
        println!("\n{}", output::bold("Learnings"));
        println!("  Total:            {}", learn_total);
        println!("  Total references: {}", learn_refs);

        // Breakdown by category
        let mut stmt = conn.prepare(
            "SELECT category, COUNT(*) FROM learnings GROUP BY category ORDER BY COUNT(*) DESC",
        )?;

        let categories: Vec<(Option<String>, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        if !categories.is_empty() {
            println!("  By category:");
            for (cat, count) in categories {
                let cat_name = cat.unwrap_or_else(|| "uncategorized".to_string());
                println!("    {}: {}", cat_name, count);
            }
        }
    }

    println!("\n{}", "=".repeat(60));
    Ok(())
}
