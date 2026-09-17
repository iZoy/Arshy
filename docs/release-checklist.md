# v0.1.0-alpha.1 public-preview checklist

## Required before the tag

- [ ] Version and public docs consistently name v0.1.0-alpha.1.
- [ ] Format, clippy, all tests, docs, coverage, and package inspection pass.
- [ ] Text and structured MCP clients complete execute, query, and raw retrieval.
- [ ] Codex and Claude Code each register in a new session and execute a real command.
- [ ] Exit 1, late errors, invalid UTF-8, long lines, large output, timeout, and cancel pass.
- [ ] All four release targets are run-tested; cross-compilation alone does not count.
- [ ] A clean environment completes the documented installation and MCP setup.
- [ ] Package contents and public Git history contain no private logs or credentials.
- [ ] SECURITY, known issues, rollback, retained data, and feedback links are current.

## Publish

1. Create and push the v0.1.0-alpha.1 tag from a reviewed clean commit.
2. Let every platform job build, test, attest, and upload its artifact.
3. Let the final job verify all four archives and checksums.
4. Review the generated draft release and attached assets.
5. Publish it manually as a prerelease.

Do not publish to crates.io or Homebrew for this preview.

## After publication

- Complete one installation from the public release URL.
- Post the English and Chinese launch copy.
- Triage twice per week and publish at most one routine alpha per week.
- Withdraw recommendation of a release affected by a P0 issue.
