# Commenting and Helper Rules

This is the baseline code-hygiene policy for Agent Ruler.

## Goals

- Keep security boundaries explicit and reviewable.
- Keep orchestration flow understandable without reading the whole repo.
- Keep large files manageable by extracting cohesive helper modules.

## Commenting Rules

1. Prefer signal over noise.
   - Use `///` doc comments on public structs/enums/functions.
   - Use `//` only for non-obvious "why", invariants, or boundary rationale.
   - Avoid repeating obvious code behavior.

2. Surface assumptions and non-goals.
   - For host vs managed state, approvals/receipts semantics, and detached lifecycle behavior, state the invariant and intended outcome.

3. Keep comments behavior-accurate.
   - Update comments in the same change that updates behavior.
   - Do not leave speculative comments that are not enforced by code.

4. Comment security-relevant boundaries explicitly when touched.
   - Host vs managed runtime boundary.
   - Redaction boundaries for agent-visible feeds.
   - Non-interactive defaults and why they are safe.

5. Guard noise by design.
   - Comment at logical-block granularity, not line-by-line.
   - Prefer one concise invariant comment over many trivial comments.

## Helper Split Rules

1. Extract helpers under `src/helpers/` when files mix concerns or get large.
2. Keep orchestration in the owner file; move parsing/mutation details to helpers.
3. Use feature-tree naming (for example `helpers/commands/ui.rs`, `helpers/runners/openclaw/setup_config.rs`).
4. Add provenance module headers in extracted helpers:
   - source/origin file,
   - owning flow/function,
   - critical invariants.
5. Prefer narrow helper APIs (`pub(crate)` where possible).

## Runtime Safety Notes

- Delete/cleanup code must enforce runtime-root scoping checks.
- Setup/import paths must not mutate host runner homes.
- Detached process lifecycle must persist deterministic stop metadata (PID/log records).

## Review Checklist

1. Public APIs touched have `///` docs when needed.
2. Security-relevant behavior has explicit invariant comments.
3. Large-file edits considered helper extraction where appropriate.
4. New helper modules include source-provenance headers.
5. `cargo fmt` and `cargo test` pass.
