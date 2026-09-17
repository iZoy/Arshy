# Reproducible Agent diagnostic loop

This demo contains one intentional Rust type error. Copy the fixture outside
the repository so the source fixture remains unchanged:

~~~bash
demo_dir="$(mktemp -d)"
cp -R docs/tutorials/demo-rust-failure/. "$demo_dir/"
cd "$demo_dir"
~~~

In a fresh MCP-enabled Agent session, give this prompt:

~~~text
Use arshy_exec to run cargo check in the current project. Read the primary
diagnostic and location. Retrieve the original output with arshy_task raw,
fix only the reported type mismatch, and run cargo check again.
~~~

Expected sequence:

1. arshy_exec returns failed, exit_code 101, a task ID, and a primary
   diagnostic pointing to src/main.rs.
2. arshy_task with action raw returns the captured compiler output.
3. The Agent changes the returned integer to a String, for example by calling
   to_string.
4. The second arshy_exec call returns completed and exit_code 0.

Record whether the Agent saw the diagnostic without another query, how many
tool calls it made, and whether it repeated a command. Do not submit the
temporary directory or output unless you have reviewed it.
