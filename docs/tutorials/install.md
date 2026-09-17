# Install v0.1.0-alpha.1

Arshy supports macOS and Linux. The installer requires Bash and a checksum
tool, and installs only the two release binaries.

~~~bash
curl -fsSL https://raw.githubusercontent.com/iZoy/Arshy/v0.1.0-alpha.1/install/install.sh \
  | bash -s -- --version v0.1.0-alpha.1
arshy doctor
~~~

Use --dry-run to inspect the target and URL, or --install-dir to select a
different binary directory. The installer stages both binaries and restores
the previous pair if replacement fails.

After installation, run arshy mcp config --format prompt in the intended
project and give its complete output to the Agent. Verify the registered entry,
then restart the client or create a new session before checking for the tools.

To roll back, rerun the installer with a previous release tag. To uninstall,
remove the MCP entry and both binaries. Task history remains in the configured
store directory and must be reviewed and removed separately if desired.
