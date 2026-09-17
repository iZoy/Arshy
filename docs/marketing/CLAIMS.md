# Public-preview claims for v0.1.0-alpha.1

## Safe claims

- Arshy exposes one generic MCP server and a client-neutral setup Prompt;
  client-specific configuration remains owned by each Agent.
- Arshy parses command output into structured events with severity, locations,
  error codes, deduplication, and optional source context.
- Arshy reports versioned, component-only quality measurements (`quality-v1`)
  on explicit analytics commands.

## Claims we do not make

We do not claim a fixed token-saving percentage. Quality components are not a tokenizer model,
not a billing estimate, and not a guarantee for a particular agent or model.
Any public benchmark must include the command corpus, platform, parser
version, sample size, raw counters, and schema version.

## Suggested launch copy

> Arshy is a local standard MCP command layer for AI agents. It turns noisy
> terminal output into structured, traceable diagnostics and retains original
> output locally. The public preview validates repeated value in real projects.
