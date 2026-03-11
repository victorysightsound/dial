//! Example of configuring a custom validation pipeline.

use dial_core::Engine;

#[tokio::main]
async fn main() -> dial_core::Result<()> {
    let engine = Engine::init("validator-demo", None, false).await?;

    // Add validation pipeline steps (ordered by sort_order)
    engine.pipeline_add("format", "cargo fmt --check", 0, false, Some(30)).await?;
    engine.pipeline_add("clippy", "cargo clippy -- -D warnings", 1, false, Some(120)).await?;
    engine.pipeline_add("build", "cargo build --workspace", 2, true, Some(300)).await?;
    engine.pipeline_add("test", "cargo test --workspace", 3, true, Some(600)).await?;

    // List the pipeline
    let steps = engine.pipeline_list().await?;
    println!("Validation pipeline ({} steps):", steps.len());
    for step in &steps {
        println!(
            "  [{}] {} ({}): {}{}",
            step.sort_order,
            step.name,
            if step.required { "required" } else { "optional" },
            step.command,
            step.timeout_secs
                .map(|t| format!(" ({}s timeout)", t))
                .unwrap_or_default(),
        );
    }

    println!("\nPipeline configured! Steps run in order.");
    println!("Required steps abort on failure. Optional steps continue.");
    Ok(())
}
