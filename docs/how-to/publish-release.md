# Publish a prerelease

This checklist is for maintainers. It does not push, tag, or publish by itself.

## Before the first public tag

1. Merge `portfolio-prep` (or the release branch) to `main`.
2. Confirm `Cargo.toml` `version` matches the intended tag without the leading
   `v` (for example `0.1.0-alpha.1` ↔ `v0.1.0-alpha.1`).
3. Confirm Linux archives will be named
   `arshy-<tag>-x86_64-unknown-linux-musl.tar.gz` and
   `arshy-<tag>-aarch64-unknown-linux-musl.tar.gz` (see
   `.github/workflows/release.yml` and `install/install.sh`).

### Historical remote tag `v0.1.0`

Origin currently has an annotated tag `v0.1.0` on the 2026-08-04 initial
commit. There is no matching GitHub Release. Leaving it in place will confuse
users when a real `v0.1.0` ships later.

**Do not run these until you have explicitly confirmed deletion.** They rewrite
remote tags:

~~~bash
# Inspect first
git fetch --tags origin
git show -s --format='%H %ci %s' v0.1.0
gh release view v0.1.0 -R iZoy/Arshy || true

# Delete remote + local tag (irreversible for anyone who already fetched it)
git push origin :refs/tags/v0.1.0
git tag -d v0.1.0
~~~

If anyone may already depend on that tag name, prefer renaming instead of
deleting (create `v0.1.0-dev.initial` on the same commit, then delete
`v0.1.0`).

## Cut the prerelease

~~~bash
git checkout main
git pull
git tag -a v0.1.0-alpha.1 -m "v0.1.0-alpha.1"
git push origin v0.1.0-alpha.1
gh run watch -R iZoy/Arshy
# Inspect the draft, then:
gh release edit v0.1.0-alpha.1 -R iZoy/Arshy --draft=false
~~~

Release CI builds four archives (two macOS, two Linux musl), checks Linux
binaries are statically linked, attests provenance, and opens a draft
prerelease only after all four checksums verify.

## crates.io and MCP Registry

MCP Registry `server.json` uses `registryType: cargo`. Publish the crate
before registering the server:

~~~bash
cargo publish
# then, with the official publisher:
mcp-publisher login github
mcp-publisher publish
~~~

The package README must keep the visible `mcp-name: io.github.iZoy/arshy`
token (crates.io strips HTML comments).
