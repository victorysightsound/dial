use dial_core::event::{Event, EventHandler};
use dial_core::output;

/// CLI event handler that prints events to the terminal.
pub struct CliEventHandler;

impl EventHandler for CliEventHandler {
    fn handle(&self, event: &Event) {
        match event {
            Event::TaskAdded {
                id, description, ..
            } => {
                output::print_success(&format!("Added task #{}: {}", id, description));
            }
            Event::TaskCompleted { id } => {
                output::print_success(&format!("Task #{} marked as completed.", id));
            }
            Event::TaskBlocked { id, reason } => {
                println!(
                    "{}",
                    output::yellow(&format!("Task #{} blocked: {}", id, reason))
                );
            }
            Event::TaskCancelled { id } => {
                println!("{}", output::dim(&format!("Task #{} cancelled.", id)));
            }
            Event::TaskUnblocked { id } => {
                output::print_success(&format!("Task #{} auto-unblocked.", id));
            }
            Event::TaskDependencyAdded {
                task_id,
                depends_on_id,
            } => {
                output::print_success(&format!(
                    "Task #{} now depends on #{}",
                    task_id, depends_on_id
                ));
            }
            Event::TaskDependencyRemoved {
                task_id,
                depends_on_id,
            } => {
                output::print_success(&format!(
                    "Removed dependency: #{} no longer depends on #{}",
                    task_id, depends_on_id
                ));
            }
            Event::IterationStarted {
                iteration_id: _,
                task,
                attempt,
                max_attempts,
            } => {
                println!("{}", output::bold(&"=".repeat(60)));
                println!("{}", output::bold(&format!("Iteration: Task #{}", task.id)));
                println!("Description: {}", task.description);
                println!("{}", output::bold(&"=".repeat(60)));
                println!("Attempt {} of {}", attempt, max_attempts);
            }
            Event::IterationCompleted {
                iteration_id,
                task_id,
                commit_hash,
            } => {
                if let Some(hash) = commit_hash {
                    println!(
                        "{}",
                        output::green(&format!("Committed: {}", &hash[..8.min(hash.len())]))
                    );
                }
                println!(
                    "{}",
                    output::green(&format!("Iteration #{} completed!", iteration_id))
                );
                println!(
                    "{}",
                    output::green(&format!("Task #{} marked as completed.", task_id))
                );
            }
            Event::IterationFailed {
                iteration_id,
                task_id: _,
                error,
            } => {
                println!(
                    "{}",
                    output::red(&format!("Iteration #{} failed: {}", iteration_id, error))
                );
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
                println!(
                    "{}",
                    output::red(&format!("Validation failed: {}", preview))
                );
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
            Event::LearningAdded {
                id,
                description,
                category,
            } => {
                let cat_str = category.as_deref().unwrap_or("uncategorized");
                output::print_success(&format!("Learning #{} [{}]: {}", id, cat_str, description));
            }
            Event::LearningDeleted { id } => {
                println!("{}", output::dim(&format!("Learning #{} deleted.", id)));
            }
            Event::FailureRecorded { failure_id, .. } => {
                println!(
                    "{}",
                    output::red(&format!("Recorded failure #{}", failure_id))
                );
            }
            Event::SolutionFound {
                description,
                confidence,
            } => {
                println!(
                    "{}",
                    output::yellow(&format!(
                        "Solution (confidence {:.2}): {}",
                        confidence, description
                    ))
                );
            }
            Event::SolutionSuggested {
                failure_id,
                solutions,
            } => {
                println!(
                    "{}",
                    output::yellow(&format!(
                        "Auto-suggested {} solution(s) for failure #{}",
                        solutions.len(),
                        failure_id
                    ))
                );
                for (_, desc, conf) in solutions {
                    println!("  - KNOWN FIX (confidence: {:.2}): {}", conf, desc);
                }
            }
            Event::ConfigSet { key, value } => {
                output::print_success(&format!("Config set: {} = {}", key, value));
            }
            Event::ApprovalRequired {
                iteration_id,
                task_id,
                diff_summary,
            } => {
                println!("{}", output::bold("Approval Required"));
                println!("{}", "=".repeat(60));
                println!("Iteration #{} for task #{}", iteration_id, task_id);
                println!("\n{}", diff_summary);
                println!("\nRun `dial approve` to accept or `dial reject \"reason\"` to reject.");
            }
            Event::Approved { iteration_id } => {
                output::print_success(&format!("Iteration #{} approved.", iteration_id));
            }
            Event::Rejected {
                iteration_id,
                reason,
            } => {
                println!(
                    "{}",
                    output::yellow(&format!("Iteration #{} rejected: {}", iteration_id, reason))
                );
            }
            Event::StepStarted {
                name,
                command,
                required,
            } => {
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
            Event::StepPassed {
                name,
                duration_secs,
            } => {
                println!(
                    "    {}",
                    output::green(&format!("{} passed ({:.1}s)", name, duration_secs))
                );
            }
            Event::StepFailed {
                name,
                required,
                output: out,
                duration_secs,
            } => {
                if *required {
                    println!(
                        "    {}",
                        output::red(&format!("{} FAILED ({:.1}s)", name, duration_secs))
                    );
                    let preview = if out.len() > 200 {
                        &out[..200]
                    } else {
                        out.as_str()
                    };
                    if !preview.is_empty() {
                        println!("    {}", output::dim(preview));
                    }
                } else {
                    println!(
                        "    {}",
                        output::dim(&format!(
                            "{} failed (optional, {:.1}s)",
                            name, duration_secs
                        ))
                    );
                }
            }
            Event::StepSkipped { name, reason } => {
                println!(
                    "    {}",
                    output::dim(&format!("{} skipped: {}", name, reason))
                );
            }
            Event::PrdImported { files, sections } => {
                output::print_success(&format!(
                    "Imported {} sections from {} files into prd.db",
                    sections, files
                ));
            }
            Event::WizardPhaseStarted { phase, name } => {
                println!("\n{}", output::bold(&format!("Phase {}: {}", phase, name)));
                println!("{}", "─".repeat(40));
            }
            Event::WizardPhaseCompleted { phase, name } => {
                println!(
                    "{}",
                    output::green(&format!("Phase {}: {} complete", phase, name))
                );
            }
            Event::WizardCompleted {
                sections_generated,
                tasks_generated,
            } => {
                println!("\n{}", output::bold("Wizard Complete"));
                println!("{}", "=".repeat(40));
                output::print_success(&format!("Generated {} PRD sections", sections_generated));
                output::print_success(&format!("Created {} linked tasks", tasks_generated));
            }
            Event::WizardPaused { phase } => {
                if *phase == 0 {
                    println!("{}", output::yellow("Wizard paused. Resume with 'dial spec wizard --resume' or 'dial new --resume'."));
                } else {
                    println!("{}", output::yellow(&format!(
                        "Wizard paused at phase {}. Resume with 'dial spec wizard --resume' or 'dial new --resume'.",
                        phase
                    )));
                }
            }
            Event::WizardResumed { phase } => {
                if *phase == 0 {
                    println!("{}", output::green("Resuming wizard"));
                } else {
                    println!(
                        "{}",
                        output::green(&format!("Resuming wizard from phase {}", phase))
                    );
                }
            }
            Event::TermAdded {
                canonical,
                category,
            } => {
                output::print_success(&format!("Term added: {} [{}]", canonical, category));
            }
            Event::TaskReviewCompleted {
                tasks_kept,
                tasks_added,
                tasks_removed,
            } => {
                output::print_success(&format!(
                    "Task review complete: {} kept, {} added, {} removed",
                    tasks_kept, tasks_added, tasks_removed
                ));
            }
            Event::TaskSplit {
                original,
                into_count,
            } => {
                output::print_success(&format!(
                    "Split task into {} sub-tasks: {}",
                    into_count,
                    if original.len() > 60 {
                        &original[..60]
                    } else {
                        original
                    }
                ));
            }
            Event::TaskSizingCompleted {
                small,
                medium,
                large,
                splits,
                rewrites,
                merges,
            } => {
                output::print_success(&format!(
                    "Sizing: {}S {}M {}L | {} splits, {} rewrites, {} merges",
                    small, medium, large, splits, rewrites, merges
                ));
            }
            Event::BuildTestConfigured {
                build_cmd,
                test_cmd,
                pipeline_steps,
            } => {
                output::print_success(&format!(
                    "Build/test configured: build='{}', test='{}', {} pipeline steps",
                    build_cmd, test_cmd, pipeline_steps
                ));
            }
            Event::TestCoverageConfigured {
                test_tasks_added,
                pipeline_steps,
            } => {
                output::print_success(&format!(
                    "Added {} test tasks, configured {} pipeline steps",
                    test_tasks_added, pipeline_steps
                ));
            }
            Event::IterationModeSet { mode } => {
                output::print_success(&format!("Iteration mode set: {}", mode));
            }
            Event::LaunchReady {
                project_name,
                task_count,
                build_cmd,
                test_cmd,
                iteration_mode,
                ai_cli,
            } => {
                println!("\n{}", output::bold("Launch Ready"));
                println!("{}", "=".repeat(40));
                println!("  Project:        {}", output::bold(project_name));
                println!("  Tasks:          {}", task_count);
                println!("  Build command:  {}", build_cmd);
                println!("  Test command:   {}", test_cmd);
                println!("  Iteration mode: {}", iteration_mode);
                println!("  AI CLI:         {}", ai_cli);
                println!("{}", "=".repeat(40));
                println!();
                output::print_success(
                    "Project configured. Run `dial auto-run` to start autonomous iteration.",
                );
            }
            Event::CheckpointCreated {
                iteration_id,
                checkpoint_id,
            } => {
                println!(
                    "{}",
                    output::dim(&format!(
                        "Checkpoint '{}' created (iteration #{})",
                        checkpoint_id, iteration_id
                    ))
                );
            }
            Event::CheckpointRestored { iteration_id } => {
                println!(
                    "{}",
                    output::yellow(&format!(
                        "Checkpoint restored for iteration #{}",
                        iteration_id
                    ))
                );
            }
            Event::CheckpointDropped { iteration_id } => {
                println!(
                    "{}",
                    output::dim(&format!(
                        "Checkpoint dropped (iteration #{} passed)",
                        iteration_id
                    ))
                );
            }
            Event::ChronicFailureDetected {
                task_id,
                total_failures,
                total_attempts,
            } => {
                println!(
                    "{}",
                    output::red(&format!(
                        "Chronic failure: task #{} has {} failures across {} attempts",
                        task_id, total_failures, total_attempts
                    ))
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
