# Commenting and Helper Split Rules

This document defines the baseline code hygiene rules for Agent Ruler contributors.

## Goals

- Keep security boundaries explicit and reviewable.
- Make control flow understandable for a new contributor without reading the whole repository.
- Keep large files manageable by extracting cohesive helper modules.

## Commenting Rules

- Prefer high-signal comments over dense commentary.
- Explain intent, invariants, and security boundaries.
- Explain non-obvious control flow and edge-case handling.
- Use `///` doc comments for public structs/enums/functions.
- Use `//` comments inside functions only when the "why" is not obvious from code.
- Keep comments accurate and update them in the same change as behavior updates.

## What Not to Comment

- Do not restate obvious statements (for example, "increment counter").
- Do not comment every line.
- Do not add speculative comments that are not enforced by code.

## Security-Focused Comment Expectations

Document these explicitly where relevant:

- Host vs managed runtime boundary.
- Non-goals (what we intentionally do not support or mutate).
- Non-interactive defaults and why they are safe.
- Approval/receipt semantics and how timeline visibility is preserved.
- Redaction boundaries for agent-visible status feeds.

## Helper Split Rules

When a file grows or mixes concerns, extract helpers under `src/helpers/` following existing hierarchy.

- Keep orchestration in the original owner file.
- Move transformation/mutation/parsing details into helper modules.
- Name helper paths to show origin and domain, for example:
  - `src/helpers/ui/openclaw_tool_preflight.rs`
  - `src/helpers/runners/openclaw/setup_config.rs`
- At the top of each extracted helper file, include a module header that states:
  - source file it came from,
  - owning flow/function to verify when behavior changes,
  - critical invariants.

## Split Size and Scope Guidance

- Prefer small, cohesive helper modules over one large "utils" dump.
- Split by behavior boundary (policy mediation, config mutation, diagnostics, etc.).
- Avoid cross-domain helpers that depend on unrelated modules.
- Keep helper APIs narrow (`pub(crate)` when possible).

## Runtime Safety Rules

- Any delete/cleanup path must enforce runtime-root scoping checks.
- Any setup/import path must avoid writing to host OpenClaw home.
- Any detached process lifecycle must persist enough state for deterministic stop behavior.

## Review Checklist

Before merging:

1. Public APIs have `///` docs.
2. New security-relevant behavior has explicit invariant comments.
3. Large-file changes considered helper extraction where appropriate.
4. Extracted helpers include source-origin module header.
5. `cargo fmt` and `cargo test` pass.
