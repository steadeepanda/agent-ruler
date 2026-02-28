# Commenting Guidelines
It's not the bible but it helps keeps things organized and easy to understand

1. **Prefer signal over noise.** Doc comments (`///`) go on public structs, enums, and helper functions to explain intent, invariants, or security context. Inline comments (`//`) should explain why non-obvious control flow exists, why certain values are hard-coded, or why we tolerate errors. Avoid restating what the code already says.

2. **Surface assumptions and non-goals.** Whenever a function enforces a security boundary, touches host vs. managed state, or deals with non-interactive CLI control flow (runner guards, gateway lifecycle, approvals), mention the desired outcome and the invariants you rely on.

3. **Helper files get provenance comments.** New helper modules should reside under `src/helpers/…` to keep `src/agent_ruler.rs` and other large coordination files concise. At the top of each helper file call out which file the logic was extracted from so anyone refactoring knows why it lives there.

4. **Keep helper names expressive.** Helper modules should mirror the feature tree (for example `helpers/commands/ui.rs` for UI lifecycle helpers, `helpers/runners/openclaw` for runner-specific helpers). Prefix helper names with the originating file name when it makes reasoning or renaming easier.

5. **Document splits in the codebase.** When you split logic into a helper, leave a short comment linking the helper back to the original file so future readers can trace behavior changes quickly.

6. **Guard noise by design.** Do not sprinkle trivial comments or comment every line. Instead, comment once per logical block when the reason for the code is not obvious, or to highlight invariants that must hold for security (e.g. expected PID records, plugin wiring, preflight decision flows).
