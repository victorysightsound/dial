# DIAL Writable Worker Enforcement

Version: 4.2.8
Status: Specification
Phase: writable-worker-enforcement

---

## 1. Overview

DIAL currently depends on spawned subagents to execute implementation tasks.
In a recent autonomous run, the spawned Codex worker inherited a read-only
sandbox and could not write changes back to the workspace. The worker then
blocked repeatedly instead of completing the task.

This spec makes writable worker access an explicit runtime requirement.
The orchestrator must verify that a spawned worker can write to the active
workspace before the task starts. If the worker cannot write, DIAL must stop
immediately, mark the task blocked with a clear reason, and avoid repeated
retry loops.

This is not a prompt-only change. The runtime must enforce it.

### Goals

- Prevent read-only subagents from entering the task execution path
- Fail fast before task implementation starts if a worker cannot write
- Keep the operator experience explicit and predictable
- Support Codex, Claude, Gemini, and other subagent backends consistently
- Preserve the existing DIAL loop behavior when a writable worker is available

### Non-Goals

- Rework DIAL into a different orchestration model
- Remove sandboxing from every backend universally
- Replace backend-specific execution adapters
- Add feature work unrelated to worker write enforcement

---

## 2. Problem Statement

The current DIAL loop assumes the spawned worker can modify the workspace.
That assumption is not always true.

Observed failure mode:

- DIAL spawns a subagent
- The subagent runs in a read-only sandbox
- The patch tool rejects all edits
- The subagent blocks or retries
- The task stalls without useful progress

Prompt text alone cannot prevent this. A backend may still inherit a
read-only environment even when the prompt instructs the model to write files.

---

## 3. Required Behavior

### 3.1 Writable Worker Guarantee

Before any implementation task is handed to a subagent, DIAL must verify that
the worker can write to the workspace.

If write access is unavailable:

- the task must not start
- the task must be marked `blocked`
- the blockage reason must explain that the worker environment is read-only
- auto-run must stop instead of retrying the same worker configuration

### 3.2 Recovery Behavior

If the selected backend or runner can be reconfigured to a writable mode:

- DIAL should allow a retry only after the configuration changes
- the retry must use the new writable configuration
- the previous blocked state should remain recorded for auditability

### 3.3 Backend-Agnostic Policy

The same rule applies to:

- Codex workers
- Claude workers
- Gemini workers
- any future agent backend used by DIAL

The orchestrator must not assume backend-specific sandbox behavior.

---

## 4. Design Principles

- Enforce capability checks in the host/orchestrator, not only in the prompt
- Fail before task execution, not after several failed retries
- Prefer deterministic probes over inference from tool errors
- Keep the blocked reason human-readable and actionable
- Preserve backward compatibility where possible, but do not keep silent failure

---

## 5. Implementation Plan

### 5.1 Worker Capability Probe

Add a preflight check that verifies the worker can write to the workspace.

Recommended probe:

- create a temporary file inside `.dial/` or another DIAL-owned writable path
- write a short marker payload
- read it back
- delete it

Requirements:

- the check must run before spawning the implementation worker
- the check must be cheap and deterministic
- the check must fail if the workspace is read-only
- the check must not modify user code

Suggested API shape:

- `probe_worker_write_access(workdir: &Path) -> Result<WorkerAccess>`
- `WorkerAccess::Writable`
- `WorkerAccess::ReadOnly { reason: String }`

### 5.2 Execution Gate

The orchestrator must gate task execution on the probe result.

If writable:

- proceed with prompt generation and worker spawn

If read-only:

- do not spawn the implementation worker
- mark the task blocked
- store the reason in `blocked_by`
- emit a blocked event for the audit trail

### 5.3 Prompt Hardening

Update the generated prompt to state:

- the worker must be able to write to the workspace
- if the environment is read-only, the worker must stop and report blocked
- the worker should not attempt to continue in a read-only mode

This prompt language is a secondary safeguard only.

### 5.4 Backend Adapter Policy

Each AI backend adapter must expose whether the spawned worker is writable.

Backend adapters should support one of these outcomes:

- writable subprocess
- explicitly read-only subprocess
- unknown, which must be treated as non-writable until probed

If a backend cannot guarantee writability, the orchestrator must probe before
the task starts.

### 5.5 Retry Policy

Retry behavior must change:

- write-probe failure is not a normal implementation retry
- the task should not consume the usual 3-attempt budget on a read-only worker
- the task remains blocked until the environment is fixed

This prevents the system from burning retries on a configuration problem.

---

## 6. Backend-Specific Notes

### Codex

- The Codex worker must not be launched in a read-only sandbox for DIAL
  implementation tasks.
- If the Codex runner or CLI integration inherits a restrictive sandbox, DIAL
  must fail the task before execution.
- The task prompt should explicitly say that read-only execution is not
  acceptable.

### Claude

- Claude subagents must be treated the same way.
- A successful prompt launch is not enough; write access must be verified.

### Gemini

- Gemini worker mode must obey the same probe and gating rules.

### Future Backends

- Any new backend must implement the same writable-worker contract before it
  can be used for autonomous implementation tasks.

---

## 7. Logging and Audit

When a worker is blocked for write access reasons, DIAL must record:

- task id
- backend name
- probe result
- blocked reason
- timestamp
- whether the environment was explicitly read-only or merely unverified

Recommended blocked reason:

- `worker workspace is read-only; implementation tasks require write access`

If the worker becomes writable later, the later successful run should remain a
separate recorded attempt.

---

## 8. Testing

### Unit Tests

- probe succeeds when the workspace is writable
- probe fails when the workspace is read-only
- execution gate blocks before worker spawn on read-only access
- blocked reason is stable and human-readable
- retries are not consumed on a preflight write failure

### Integration Tests

- mock backend returns writable worker, task starts normally
- mock backend returns read-only worker, task is blocked immediately
- auto-run stops instead of retrying the same read-only worker

### Regression Test

- reproduce the previous failure pattern using a mock read-only worker
- verify DIAL blocks the task on preflight rather than entering the edit loop

---

## 9. Acceptance Criteria

DIAL is considered fixed when all of the following are true:

- read-only worker execution is detected before task implementation starts
- blocked tasks record a clear reason and do not spin in retry loops
- writable workers proceed normally
- the behavior is consistent across backends
- tests cover both writable and read-only worker modes

---

## 10. Recommended DIAL Task Split

1. Add a worker write-access probe to the orchestrator.
2. Gate task execution on probe results before subagent spawn.
3. Update subagent prompt templates to require writable workspaces.
4. Record blocked reasons and audit metadata for read-only workers.
5. Add unit and integration tests for writable and read-only worker modes.
6. Verify Codex, Claude, and Gemini adapter behavior under the new gate.

---

## 11. Stop Conditions

Stop and split a follow-up spec if implementation reveals:

- a backend that cannot support a writable worker at all
- a sandbox model that needs backend-specific handling beyond the shared gate
- a command runner that cannot reliably probe workspace write access
- a retry system that needs broader redesign to separate config failures from
  implementation failures
