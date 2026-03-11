# Contributing to DIAL

## Getting Started

> **Note:** The crate is published on crates.io as `dial-cli` (because `dial` was taken). The binary name remains `dial`.

```bash
git clone https://github.com/victorysightsound/dial.git
cd dial/dial
cargo build
cargo test
```

## Development

### Building

```bash
cargo build              # Debug build
cargo build --release    # Optimized release build
```

### Running Tests

```bash
cargo test               # All tests
cargo test patterns      # Tests matching "patterns"
cargo test -- --nocapture  # Show println output
```

### Testing Locally

```bash
# Build and test against a sample project
cargo build
mkdir -p /tmp/test-project && cd /tmp/test-project
git init
/path/to/dial/target/debug/dial init --phase test
/path/to/dial/target/debug/dial config set build_cmd "echo build ok"
/path/to/dial/target/debug/dial config set test_cmd "echo test ok"
/path/to/dial/target/debug/dial task add "Test task" -p 1
/path/to/dial/target/debug/dial iterate
/path/to/dial/target/debug/dial validate
/path/to/dial/target/debug/dial stats
```

## Project Structure

See [Architecture](docs/architecture.md) for a detailed walkthrough of the codebase.

Key locations:

| What | Where |
|------|-------|
| CLI entry point | `dial/src/main.rs` |
| Public API | `dial/src/lib.rs` |
| Database schema | `dial/src/db/schema.rs` |
| Failure patterns | `dial/src/failure/patterns.rs` |
| Signal parsing | `dial/src/iteration/orchestrator.rs` |
| Context assembly | `dial/src/iteration/context.rs` |

## Guidelines

- Run `cargo test` before submitting
- Run `cargo build` to verify compilation
- Add tests for new failure patterns or signal parsing changes
- Keep CLI output consistent with existing formatting (use `output.rs` helpers)
- New commands need a variant in `Commands` enum, a match arm in `run_command()`, and a module function

## Reporting Issues

Open an issue at [github.com/victorysightsound/dial/issues](https://github.com/victorysightsound/dial/issues) with:

- DIAL version (`dial --version`)
- Rust version (`rustc --version`)
- Operating system
- Steps to reproduce
- Expected vs actual behavior

## License

MIT. By contributing, you agree your contributions are licensed under the same terms.
