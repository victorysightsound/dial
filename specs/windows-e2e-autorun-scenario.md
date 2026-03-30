# Windows End-to-End Auto-Run Scenario

## Goal

Validate the complete DIAL loop on a small native Windows project:

1. Run the guided wizard
2. Generate a compact, concrete backlog
3. Execute the backlog with `dial auto-run --cli codex`
4. Confirm the resulting repository matches the requested scope

## Repository Context

The target repository is an existing tiny Node.js project named `mini-note-formatter`.

It already contains:

- `package.json` with working `npm test` and `npm run build` scripts
- `src/noteFormatter.js` with baseline note-formatting behavior
- `src/cli.js` as a placeholder entry point
- `test/noteFormatter.test.js` with baseline tests

This is not a web app, desktop app, service, database project, or framework migration.
It is a very small command-line utility and library.

## Requested Outcome

Implement exactly these capabilities for the MVP:

1. Add note status formatting:
   - input status `todo` renders `[ ]`
   - input status `done` renders `[x]`
   - if status is missing, render no checkbox prefix

2. Normalize tags:
   - convert tags to lowercase
   - trim whitespace
   - remove duplicates
   - preserve first-seen order after normalization

3. Finish the CLI:
   - read one JSON note object from stdin
   - print formatted output to stdout
   - exit with a non-zero code and a clear error message on invalid JSON

4. Expand automated tests to cover the new behavior

## Hard Constraints

- Use plain Node.js only
- Do not add dependencies
- Do not add TypeScript, bundlers, databases, UI frameworks, or config systems
- Do not rename existing files
- Keep the implementation within these files unless absolutely necessary:
  - `package.json`
  - `src/noteFormatter.js`
  - `src/cli.js`
  - `test/noteFormatter.test.js`
- Keep `npm test` as the test command
- Keep `npm run build` as the build command
- Prefer 3 to 5 implementation tasks total

## Acceptance Checks

- `npm test` passes
- `npm run build` passes
- Formatting a note with status `todo` prefixes the title with `[ ]`
- Formatting a note with status `done` prefixes the title with `[x]`
- Duplicate mixed-case tags such as `Bug`, ` bug `, and `BUG` become one `bug`
- `node src/cli.js` accepts JSON from stdin and prints the formatted note
- Invalid JSON on stdin causes a non-zero exit code and a human-readable error
