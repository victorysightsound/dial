use dial_core::event::{Event, EventHandler};
use dial_core::output;

/// CLI event handler that prints events to the terminal.
pub struct CliEventHandler;

impl EventHandler for CliEventHandler {
    fn handle(&self, event: &Event) {
        match event {
            Event::TaskAdded { id, description, .. } => {
                output::print_success(&format!("Added task #{}: {}", id, description));
            }
            Event::TaskCompleted { id } => {
                output::print_success(&format!("Task #{} marked as completed.", id));
            }
            Event::TaskBlocked { id, reason } => {
                println!("{}", output::yellow(&format!("Task #{} blocked: {}", id, reason)));
            }
            Event::TaskCancelled { id } => {
                println!("{}", output::dim(&format!("Task #{} cancelled.", id)));
            }
            Event::TaskUnblocked { id } => {
                output::print_success(&format!("Task #{} auto-unblocked.", id));
            }
            Event::TaskDependencyAdded { task_id, depends_on_id } => {
                output::print_success(&format!("Task #{} now depends on #{}", task_id, depends_on_id));
            }
            Event::TaskDependencyRemoved { task_id, depends_on_id } => {
                output::print_success(&format!("Removed dependency: #{} no longer depends on #{}", task_id, depends_on_id));
            }
            Event::IterationStarted { iteration_id: _, task, attempt, max_attempts } => {
                println!("{}", output::bold(&"=".repeat(60)));
                println!("{}", output::bold(&format!("Iteration: Task #{}", task.id)));
                println!("Description: {}", task.description);
                println!("{}", output::bold(&"=".repeat(60)));
                println!("Attempt {} of {}", attempt, max_attempts);
            }
            Event::IterationCompleted { iteration_id, task_id, commit_hash } => {
                if let Some(hash) = commit_hash {
                    println!("{}", output::green(&format!("Committed: {}", &hash[..8.min(hash.len())])));
                }
                println!("{}", output::green(&format!("Iteration #{} completed!", iteration_id)));
                println!("{}", output::green(&format!("Task #{} marked as completed.", task_id)));
            }
            Event::IterationFailed { iteration_id, task_id: _, error } => {
                println!("{}", output::red(&format!("Iteration #{} failed: {}", iteration_id, error)));
            }
            Event::ValidationStarted { iteration_id } => {
                println!("Validating iteration #{}...", iteration_id);
            }
            Event::ValidationPassed => {
                println!("{}", output::green("Validation passed."));
            }
            Event::ValidationFailed { error_output } => {
                let preview = if error_output.len() > 200 {
                    &error_output[..200]
                } else {
                    error_output
                };
                println!("{}", output::red(&format!("Validation failed: {}", preview)));
            }
            Event::BuildStarted { command } => {
                println!("Running build: {}", command);
            }
            Event::BuildPassed => {
                println!("{}", output::green("Build passed."));
            }
            Event::BuildFailed { output: out } => {
                let preview = if out.len() > 200 { &out[..200] } else { out };
                println!("{}", output::red(&format!("Build failed: {}", preview)));
            }
            Event::TestStarted { command } => {
                println!("Running tests: {}", command);
            }
            Event::TestPassed => {
                println!("{}", output::green("Tests passed."));
            }
            Event::TestFailed { output: out } => {
                let preview = if out.len() > 200 { &out[..200] } else { out };
                println!("{}", output::red(&format!("Tests failed: {}", preview)));
            }
            Event::LearningAdded { id, description, category } => {
                let cat_str = category.as_deref().unwrap_or("uncategorized");
                output::print_success(&format!("Learning #{} [{}]: {}", id, cat_str, description));
            }
            Event::LearningDeleted { id } => {
                println!("{}", output::dim(&format!("Learning #{} deleted.", id)));
            }
            Event::FailureRecorded { failure_id, .. } => {
                println!("{}", output::red(&format!("Recorded failure #{}", failure_id)));
            }
            Event::SolutionFound { description, confidence } => {
                println!("{}", output::yellow(&format!("Solution (confidence {:.2}): {}", confidence, description)));
            }
            Event::ConfigSet { key, value } => {
                output::print_success(&format!("Config set: {} = {}", key, value));
            }
            Event::StepStarted { name, command, required } => {
                println!(
                    "{}",
                    output::dim(&format!(
                        "  Step '{}'{}: {}",
                        name,
                        if *required { "" } else { " (optional)" },
                        command,
                    ))
                );
            }
            Event::StepPassed { name, duration_secs } => {
                println!(
                    "    {}",
                    output::green(&format!("{} passed ({:.1}s)", name, duration_secs))
                );
            }
            Event::StepFailed { name, required, output: out, duration_secs } => {
                if *required {
                    println!(
                        "    {}",
                        output::red(&format!("{} FAILED ({:.1}s)", name, duration_secs))
                    );
                    let preview = if out.len() > 200 { &out[..200] } else { out.as_str() };
                    if !preview.is_empty() {
                        println!("    {}", output::dim(preview));
                    }
                } else {
                    println!(
                        "    {}",
                        output::dim(&format!("{} failed (optional, {:.1}s)", name, duration_secs))
                    );
                }
            }
            Event::StepSkipped { name, reason } => {
                println!(
                    "    {}",
                    output::dim(&format!("{} skipped: {}", name, reason))
                );
            }
            Event::Info(msg) => {
                println!("{}", msg);
            }
            Event::Warning(msg) => {
                println!("{}", output::yellow(msg));
            }
            Event::Error(msg) => {
                println!("{}", output::red(msg));
            }
        }
    }
}
