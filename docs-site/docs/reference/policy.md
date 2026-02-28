# Policy Reference

Policy file: `state/policy.yaml`

## Profiles

- `strict` (default): strongest baseline; advanced + elevation customization locked.
- `simple_user`: safest daily-driver profile with most advanced controls locked.
- `balanced`: quick-start baseline; network/elevation controls open, advanced rule matrix locked.
- `coding_nerd`: coding-oriented profile; advanced controls unlocked.
- `i_dont_care`: emergency low-friction mode (not recommended for regular use).
- `custom`: full personalization profile.

System-critical filesystem disposition is always forced to `deny` across all profiles.
Secrets disposition is always forced to `deny`.
Download->exec quarantine remains enforced as a minimum guard in every profile.

## Recommended profile selection

- Use `simple_user` if you want safety with minimal settings.
- Use `balanced` if you want a practical default and faster onboarding.
- Use `coding_nerd` for frequent development tasks with fewer interruptions.
- Use `strict` for untrusted tasks or shared machines.
- Use `i_dont_care` only for temporary unblocking.
- Use `custom` when you intentionally manage all advanced settings.

## Rules

- Filesystem zone matrix (`allow`, `deny`, `approval`)
- Network controls:
  - `default_deny`
  - `allowlist_hosts`
  - `require_approval_for_post` (approval gate for POST requests, including allowlisted hosts)
  - `invert_allowlist` / `invert_denylist` (list interpretation toggles)
- Execution controls: workspace/tmp execution guards and download->exec quarantine
- Persistence controls: deny and approval path sets
- Elevation controls:
  - `enabled`
  - `require_operator_auth`
  - `use_allowlist`
  - `allowed_packages`
  - `denied_packages` (always enforced)

## Safeguards

- `mass_delete_threshold`: file-count threshold in a single delete operation that triggers approval.

## Approvals

- `ttl_seconds`
- Pending/resolution state tracked in approvals store
- Long-poll waiting via `/api/approvals/<id>/wait`

## Notes on package policy

- If `use_allowlist = true`, only packages in `allowed_packages` can pass (unless denylisted).
- If `use_allowlist = false`, allowlist is skipped, but `denied_packages` still blocks.
- If a package appears in both lists, denylist wins.

## Notes on network list defaults

- Domain lists are unchecked by default in UI presets.
- Invert toggles decide whether checked hosts block or allow; unchecked lists do not create explicit host matches by themselves.
- GET can remain broad (whole internet) when `default_deny = false` and confinement/network posture allows it.
- POST remains more sensitive: allow only trusted domains and prefer manual human POST actions for sensitive write operations.
- All network requests are still submitted to Agent Ruler enforcement (policy + confinement + approval gates).
