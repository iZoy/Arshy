// Arshy Rhai parser example — Docker build output
//
// Drop this file into ~/.arshy/parsers/ to enable it.
// arshy will auto-detect "docker" commands and use this parser.
//
// API reference:
//   ctx.emit(event_type, severity, message)
//   ctx.emit(event_type, severity, message, file, line)
//   ctx.set(key, value) / ctx.get(key) / ctx.has(key)

fn on_line(line, ctx) {
    // Error: failed to solve
    if line.contains("ERROR") && line.contains("failed to solve") {
        ctx.emit("diagnostic", "error", line);
        ctx.set("has_error", true);
        return;
    }

    // Generic error lines (e.g., "error: ...")
    if line.starts_with("error:") || line.starts_with("ERROR: ") {
        ctx.emit("diagnostic", "error", line);
        ctx.set("has_error", true);
        return;
    }

    // Warning lines
    if line.starts_with("WARNING") || line.starts_with("WARN") {
        ctx.emit("diagnostic", "warning", line);
        return;
    }

    // Track build stages: "#1 [internal] load build definition"
    if line.starts_with("#") && line.contains("[") && line.contains("]") {
        ctx.set("current_stage", line);
    }

    // Caching hit
    if line.contains("CACHED") {
        ctx.emit("cache", "info", line);
    }

    // Successfully built image
    if line.starts_with("Successfully built") {
        ctx.set("success", true);
        ctx.emit("summary", "info", line);
    }

    // Tagged image
    if line.starts_with("Successfully tagged") {
        ctx.emit("summary", "info", line);
    }
}

fn on_complete(exit_code, ctx) {
    if exit_code != 0 {
        ctx.emit("summary", "error", "docker build failed (exit " + exit_code + ")");
    } else if !ctx.has("success") {
        // Exit 0 but no success message — unusual
        ctx.emit("summary", "info", "docker build completed (exit 0)");
    }
}
