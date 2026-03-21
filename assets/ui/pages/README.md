# WebUI Page Map

Each tab is now split into a focused file so page content can be edited in one place.

- Main
  - `main/overview.js`
  - `main/approvals.js`
  - `main/import-export.js`
- Configuration
  - `configuration/policy.js`
  - `configuration/runtime-paths.js`
  - `configuration/control-settings.js`
  - `configuration/execution-layer.js`
- Monitoring
  - `monitoring/receipts.js`
  - `monitoring/runners.js`

The include order is defined in `src/helpers/ui/pages.rs`.
