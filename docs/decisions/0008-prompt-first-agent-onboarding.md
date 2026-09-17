# ADR 0008: Prompt-first agent onboarding

- Status: accepted
- Date: 2026-08-25

## Context

Arshy exposes a client-neutral stdio MCP server. Agent clients share the
transport, but differ in registration commands, configuration scope, project
instruction files, permissions, and session reload behavior. Maintaining a
Rust adapter for every client would make the integration surface brittle and
would require Arshy releases for client configuration changes.

## Decision

Keep `arshy mcp serve` and the JSON/command configuration output as the stable
integration boundary. Add `arshy mcp config --format prompt`, which emits a
versioned, copyable English procedure. The current Agent uses its own native
MCP management interface and reports scope, conflicts, verification, rollback,
and any required human or GUI action.

The generated procedure may ask the Agent to update an existing project
instruction file only when the current project policy and user authorization
call for it. Arshy does not require a file, marker, or filename. Any such
change must preserve user content, report the file, stop on ambiguity or
conflict, and never modify PATH, shell profiles, hooks, unrelated MCP entries,
or installed software.

This is a limited revision of the earlier “no agent-specific commands” rule:
Arshy still has no client-specific runtime or installer, but it provides a thin
client-neutral onboarding guide because the Agent is best placed to know its
own configuration surface.

## Consequences

The implementation and support burden stays in the generic MCP contract and a
small, testable Prompt. Some clients will require a manual GUI or policy step;
the Prompt must report that limitation rather than claim universal one-click
configuration. Client compatibility is documented and manually verified, not
encoded as per-client Rust adapters.
