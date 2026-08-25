# v0.1.0-dev.1 internal candidate checklist

## Before tagging

- [ ] `Cargo.toml` and `Cargo.lock` report `0.1.0-dev.1`.
- [ ] README and English/Chinese docs describe generic MCP only.
- [ ] No `setup`, `hook`, `--agent`, or shell-injection path remains in the
      public CLI or installer.
- [ ] quality-v1 documentation lists component definitions and sample size;
      no aggregate score or token-saving claim remains.
- [ ] `security.allowed_cwds` is documented as a cwd guard, not a sandbox.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo clippy --all-targets -- -D warnings` passes.
- [ ] `cargo test` and integration tests pass.
- [ ] Coverage is collected in CI: whole repository and critical paths ≥75%
      for the internal candidate; raise this gate after process-level proxy
      and daemon entry-point tests are expanded.
- [ ] `cargo package --allow-dirty --no-verify` is inspected locally; publish
      only from a clean tagged checkout.

## Tag and publish

1. Merge the internal candidate to `main`; do not create a public release tag.
2. Only after explicit release approval should the version be changed to
   `0.1.0`, tagged as `v0.1.0`, and sent through the release workflow.
3. Verify every archive/checksum and the installer's `--dry-run` output.
4. Create the GitHub release only after the public-release approval gate.
5. From the clean tag, run `cargo publish --dry-run`, then publish to crates.io
   manually after reviewing the package contents.

Homebrew is intentionally deferred until the first release has stable install
telemetry and a maintained formula.
