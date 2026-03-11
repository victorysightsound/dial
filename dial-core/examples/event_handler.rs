//! Example of implementing custom event handlers.

use dial_core::{Engine, Event, EventHandler};
use std::sync::{Arc, Mutex};

/// Counts events by category.
struct MetricsCollector {
    task_events: Mutex<u64>,
    iteration_events: Mutex<u64>,
    validation_events: Mutex<u64>,
}

impl MetricsCollector {
    fn new() -> Self {
        Self {
            task_events: Mutex::new(0),
            iteration_events: Mutex::new(0),
            validation_events: Mutex::new(0),
        }
    }

    fn summary(&self) -> String {
        format!(
            "Events: tasks={}, iterations={}, validations={}",
            self.task_events.lock().unwrap(),
            self.iteration_events.lock().unwrap(),
            self.validation_events.lock().unwrap(),
        )
    }
}

impl EventHandler for MetricsCollector {
    fn handle(&self, event: &Event) {
        match event {
            Event::TaskAdded { .. }
            | Event::TaskCompleted { .. }
            | Event::TaskBlocked { .. }
            | Event::TaskCancelled { .. } => {
                *self.task_events.lock().unwrap() += 1;
            }
            Event::IterationStarted { .. }
            | Event::IterationCompleted { .. }
            | Event::IterationFailed { .. } => {
                *self.iteration_events.lock().unwrap() += 1;
            }
            Event::ValidationStarted { .. }
            | Event::ValidationPassed
            | Event::ValidationFailed { .. }
            | Event::StepPassed { .. }
            | Event::StepFailed { .. } => {
                *self.validation_events.lock().unwrap() += 1;
            }
            _ => {}
        }
    }
}

/// Logs events to a file-like sink.
struct FileLogger;

impl EventHandler for FileLogger {
    fn handle(&self, event: &Event) {
        // In production, write to a log file or external service
        eprintln!("[LOG] {:?}", event);
    }
}

#[tokio::main]
async fn main() -> dial_core::Result<()> {
    let mut engine = Engine::init("event-demo", None, false).await?;

    // Register multiple handlers - they all receive every event
    let collector = Arc::new(MetricsCollector::new());
    engine.on_event(collector.clone());
    engine.on_event(Arc::new(FileLogger));

    // Generate some events
    engine.task_add("First task", 5, None).await?;
    engine.task_add("Second task", 3, None).await?;
    let id = engine.task_add("Third task", 1, None).await?;
    engine.task_done(id).await?;

    // Show collected metrics
    println!("{}", collector.summary());

    Ok(())
}
