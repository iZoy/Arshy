// Arshy Rhai parser example — kubectl output
//
// Parses `kubectl get`, `kubectl describe`, `kubectl logs`, etc.
// Drop this file into ~/.arshy/parsers/ to enable it.
//
// API reference:
//   ctx.emit(event_type, severity, message)
//   ctx.emit(event_type, severity, message, file, line)
//   ctx.set(key, value) / ctx.get(key) / ctx.has(key)

fn on_line(line, ctx) {
    // Error from kubectl
    if line.starts_with("Error from server") || line.starts_with("error:") {
        ctx.emit("diagnostic", "error", line);
        ctx.set("has_error", true);
        return;
    }

    // Warning
    if line.starts_with("Warning:") {
        ctx.emit("diagnostic", "warning", line);
        return;
    }

    // Pod not ready / CrashLoopBackOff / ImagePullBackOff
    if line.contains("CrashLoopBackOff")
        || line.contains("ImagePullBackOff")
        || line.contains("ErrImagePull")
        || line.contains("CreateContainerConfigError")
    {
        ctx.emit("diagnostic", "error", line);
        ctx.set("has_error", true);
        return;
    }

    // Pod not ready (0/1 in READY column)
    let trimmed = line.trim();
    if trimmed.contains("0/1") && trimmed.contains("Running") {
        ctx.emit("diagnostic", "warning", line);
        return;
    }

    // Successful resource creation
    if line.contains("created") || line.contains("configured") {
        ctx.emit("resource", "info", line);
    }

    // Deleted resource
    if line.contains("deleted") {
        ctx.emit("resource", "info", line);
    }

    // Rollout status
    if line.contains("successfully rolled out") {
        ctx.emit("summary", "info", line);
        ctx.set("rollout_ok", true);
    }
}

fn on_complete(exit_code, ctx) {
    if exit_code != 0 {
        ctx.emit("summary", "error", "kubectl failed (exit " + exit_code + ")");
    }
}
