# Windows Live Smoke Scenario

## Goal

Exercise the guided DIAL wizard on Windows as a first-time user would experience it, with a realistic but bounded project brief that is rich enough to trigger all nine phases and make long-running phases visible.

## Scenario Brief

Project name: Workbench Memory

Build a local-first desktop app for AI-assisted software development notes and memory. The app should help a solo developer capture decisions, errors, commands, code snippets, and task notes while working across multiple projects. It should support semantic search over prior notes, lightweight tagging, and a timeline view so the developer can revisit why a change was made.

The first release should run well on Windows laptops without needing a cloud backend. It should use SQLite for local storage, work offline, and feel safe for developers who do not want source code or notes sent to hosted services by default. The architecture should fit the preferred stack of a shared Rust core with native desktop UI, but the UI should be kept simple for the MVP.

The product should be friendly to non-expert users. Setup should be obvious, recovery from interruptions should be clear, and the system should explain what it is doing during longer operations. The app should make it easy to capture notes from pasted text, drag-and-drop files, and short terminal transcripts.

## Why This Scenario

- It is concrete enough to produce a meaningful PRD and task plan.
- It still leaves room for the wizard to clarify scope and make visible planning decisions.
- It reflects a likely real project shape for DIAL users.
- It gives the guided UX a chance to show orientation, phase narration, checkpoint messaging, and trust-building completion output.

## Expected Wizard Signals

- Startup orientation explains that the wizard will guide the project through nine planning phases.
- The output makes it clear that `dial new` is creating the project plan and configuration, not starting autonomous implementation.
- Each phase explains its purpose in plain English.
- Long waits show heartbeat messaging instead of silent stalls.
- After phase 5, the wizard prints a planning checkpoint before moving into task refinement and command configuration.
- Completion messaging clearly states that implementation only begins if the user explicitly runs `dial auto-run`.
