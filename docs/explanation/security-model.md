# Security model

Arshy protects the local execution boundary with command filtering, optional
working-directory containment, Unix-socket peer validation, resource limits,
and audit logging. These controls address accidental or obviously dangerous
agent commands; they are not a substitute for an OS or container sandbox.

`security.allowed_cwds` is the only directory policy. When configured, Arshy
canonicalizes the requested cwd and requires it to be inside an allowed root.
An empty list is permissive. The check is fail-closed for invalid paths and
does not restrict file reads, network access, child processes, or credentials.

The short-command optimization skips persistence and parsing work only. It
does not skip filtering, cwd validation, peer checks, limits, or audit logs.
Use a dedicated OS-level sandbox when the threat model includes hostile code
or credentials.
