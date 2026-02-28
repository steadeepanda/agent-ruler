# Contributing

## Scope
Contributions should preserve the soul of this project and should keep deterministic behavior, explicit reason codes, and security-first defaults.
Follow `COMMENT_RULES.md` as best as possible to help me follow you.


## Most Important
- Do it with love. 


## Setup
1. Install Rust stable.
2. Install `bubblewrap` if developing Linux runner changes.
3. Run:
   - `cargo fmt`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test`

## Contribution Rules
- Keep policy decisions deterministic (no LLM/model calls).
- Add/maintain reason codes for any new block/approval behavior.
- Include tests for policy or confinement changes.
- Update curated public docs under `docs-site/docs/` when behavior changes.
- Prefer small, reviewable pull requests.

## Commit Hygiene
- Use descriptive commit titles.
- Include threat-model implications in PR description for security-sensitive changes.
- Never include secrets or private tokens in commits.

## Code Review Focus
- Security regressions.
- False-positive impact.
- Cross-platform safety (Linux now, Windows roadmap).
- Backward-compatible policy/schema changes.
