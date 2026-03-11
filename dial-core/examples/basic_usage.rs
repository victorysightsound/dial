//! Basic DIAL engine usage: initialize, add tasks, and run validation.

use dial_core::{Engine, Event, EventHandler};
use std::env;

struct PrintHandler;

impl EventHandler for PrintHandler {
    fn handle(&self, event: &Event) {
        println!("[event] {:?}", event);
    }
}

#[tokio::main]
async fn main() -> dial_core::Result<()> {
    let dir = env::current_dir()?;
    println!("Initializing DIAL in {:?}", dir);

    // Initialize a new DIAL project
    let mut engine = Engine::init("demo", None, false).await?;

    // Register an event handler
    engine.on_event(std::sync::Arc::new(PrintHandler));

    // Add tasks
    let t1 = engine.task_add("Implement login page", 8, None).await?;
    let t2 = engine.task_add("Write login tests", 5, None).await?;
    println!("Created tasks: #{}, #{}", t1, t2);

    // Add a dependency: tests depend on login page
    engine.task_depends(t2, t1).await?;

    // Configure build and test commands
    engine.config_set("build_cmd", "echo 'build ok'").await?;
    engine.config_set("test_cmd", "echo 'tests ok'").await?;

    // Check status
    if let Some(next) = engine.task_next().await? {
        println!("Next task: #{} - {}", next.id, next.description);
    }

    // Record a learning
    engine.learn("Always validate before committing", Some("pattern")).await?;

    println!("Done! Check .dial/ for the database.");
    Ok(())
}
