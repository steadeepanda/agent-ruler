# Agent Ruler v0.1.7 Architecture (Linux-first)

## 1. Scope and Security Objectives

Agent Ruler is a **deterministic reference monitor** plus **confinement runner** for AI agents. It contains no LLM logic in policy decisions—enforcement is purely rule-based and deterministic.

### Primary v0.1.7 Objectives

- **Deterministic decisioning** with stable reason codes
- **Detached enforcement** when processes are launched through Agent Ruler
- **Confinement-first controls** for filesystem + network + execution
- **Approval pipeline** for sensitive transitions
- **Low-operator-friction workflows** (import, stage, deliver) with explicit audit receipts
- **OWASP-aligned prompt injection defenses** via capability controls

### Design Principles

1. **Determinism above everything**: No LLM logic inside Agent Ruler
2. **Detached from agent**: Enforcement must not depend on agent cooperation
3. **Enforced interception**: All sensitive actions governed because agent runs in confinement
4. **Low latency**: Hot-path decisions are constant-ish time policy checks
5. **Low false positives**: Targeted blocks, approvals, quarantine, scoped restrictions
6. **Minimal configuration for first use**: Safe defaults, advanced knobs available

---

## 2. Threat Model

### In-Scope Attacker Behaviors

| Attack Category | Description | Defense Strategy |
|-----------------|-------------|------------------|
| Destructive writes/deletes | Attempting to modify/delete files outside workspace | Zone-based denial, approval gates |
| Secret-path access | Reading credentials, API keys, etc. | Secrets zone denial, redaction |
| Outbound exfiltration | Sending data to external endpoints | Network default-deny, allowlist, upload approval |
| Persistence setup | Installing daemons, cron jobs, startup items | Persistence path denial |
| Download→exec chains | Downloading and executing malicious code | Download taint, exec quarantine |
| Indirect prompt injection | Malicious content in web/documents triggering actions | Capability separation, approval gates |
| Tool misuse | Using available tools for destructive purposes | Operation-level guards, mass action limits |
| Multi-step attacks | Chained fetch→write→exec→persist or fetch→read→exfil | Independent step validation |

### Out-of-Scope in v0.1.7

| Limitation | Reason | Mitigation |
|------------|--------|------------|
| Processes not launched through Agent Ruler | Enforcement boundary requires our runner | Documentation, safe defaults |
| Privileged local attacker with root | Root can bypass namespace isolation | Defense-in-depth, audit logging |
| Syscall-complete mediation without kernel hooks | Operating at process/command level | Multiple independent controls |
| Memory-based exfiltration (shared memory, pipes) | Outside current interception layer | Network/filesystem focused |
| Timing/side-channel attacks | Different threat category | Not addressed in v0.1.7 |

---

## 3. Runtime Layout Strategy

### Default Runtime Root

```
~/.local/share/agent-ruler/projects/<project-key>/
```

**Rationale:**
- XDG-aligned mutable app data location
- Keeps source tree clean
- Deterministic project isolation

### Runtime Contents

```
runtime-root/
├── workspace/           # Zone 0: Agent working directory (RW)
├── shared-zone/         # Zone 2: Staged exports (approval required)
├── user_data/
│   └── logs/            # Managed runner/bridge logs
├── state/
│   ├── config.yaml      # Runtime configuration
│   ├── policy.yaml      # Policy definition
│   ├── receipts.jsonl   # Audit log (append-only)
│   ├── approvals.json   # Approval queue
│   ├── staged-exports.json  # Export state tracking
│   ├── exec-layer/      # Ephemeral execution state
│   └── quarantine/      # Quarantined files
```

`state/config.yaml` persists:
- `runtime_root` (runtime directory for this project)
- `ruler_root` (canonical install/source root used to resolve bundled assets)

### User-Facing Delivery Default

```
~/Documents/agent-ruler-deliveries/<project-name>/
```

### Manual Override

```bash
agent-ruler --runtime-dir /custom/path run <cmd>
```

---

## 4. High-Level Components

```
┌─────────────────────────────────────────────────────────────────┐
│                        User Interface Layer                      │
├──────────────────────────┬──────────────────────────────────────┤
│       CLI (main.rs)      │   Web UI (ui.rs + ui_* modules)      │
│  • init/run/status/tail  │  • Server + modular vanilla JS pages │
│  • approve/reset-exec    │  • Approvals, receipts, files        │
│  • import/export/deliver │  • Policy toggles, runtime paths     │
│  • smoke                 │  • Docs integration                  │
└──────────────────────────┴──────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Core Enforcement Layer                      │
├─────────────────────────────────────────────────────────────────┤
│  Runtime Resolver (config.rs)                                    │
│  • Single source of truth for paths                              │
│  • Project isolation                                             │
├─────────────────────────────────────────────────────────────────┤
│  Policy Engine (policy/mod.rs + policy/*)                        │
│  • Zone classification (path + metadata + operation)             │
│  • Rule evaluation (deterministic)                               │
│  • Reason code assignment                                        │
├─────────────────────────────────────────────────────────────────┤
│  Runner (runner/mod.rs + runner/*)                               │
│  • Bubblewrap confinement setup                                  │
│  • Preflight interception (rm, mv, network, interpreters)        │
│  • Command wrapping and execution                                │
├─────────────────────────────────────────────────────────────────┤
│  Transfer Gate (export_gate.rs + staged_exports.rs)              │
│  • Import: external → workspace                                  │
│  • Stage: workspace → shared-zone                                │
│  • Deliver: shared-zone → user destination                       │
└─────────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                       State Layer                                │
├─────────────────────────────────────────────────────────────────┤
│  Approvals Store (approvals.rs)                                  │
│  • Pending/approved/denied with TTL                              │
│  • Scope-keyed for targeted approvals                            │
├─────────────────────────────────────────────────────────────────┤
│  Receipts Store (receipts.rs)                                    │
│  • Append-only JSONL log                                         │
│  • Full action context + decision + zone + policy version        │
└─────────────────────────────────────────────────────────────────┘
```

---

## 5. Deterministic Policy Pipeline

```
┌──────────────────────────────────────────────────────────────┐
│                    Action Request                             │
│  • kind: FileWrite | FileDelete | Execute | NetworkEgress... │
│  • path: target path                                          │
│  • metadata: operation-specific context                       │
│  • process: pid, ppid, command, tree                          │
└──────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────┐
│                  Zone Classification                          │
│  1. Check secrets patterns (highest priority)                 │
│  2. Check system-critical patterns                            │
│  3. Check shared zone patterns                                │
│  4. Check workspace anchor                                    │
│  5. Check user-data patterns                                  │
│  6. Fallback: ownership-based (root = stricter)               │
└──────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────┐
│                   Rule Evaluation                             │
│  • Operation-specific guards (mass delete)                    │
│  • Zone disposition (allow/approval/deny)                     │
│  • Special handling: network, execution, persistence          │
│  • Download→exec quarantine check                             │
└──────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────┐
│                      Decision                                 │
│  • verdict: Allow | Deny | RequireApproval | Quarantine      │
│  • reason: stable reason code                                 │
│  • detail: human-readable explanation                         │
│  • approval_ttl_seconds: if approval required                 │
└──────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────┐
│               Approval Check (if required)                    │
│  • Look up scoped approval by scope_key                       │
│  • If approved: upgrade to Allow                              │
│  • If pending: queue for approval                             │
└──────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌──────────────────────────────────────────────────────────────┐
│                    Receipt Emission                           │
│  • timestamp, action, decision, zone                          │
│  • policy_version, policy_hash                                │
│  • diff_summary (when applicable)                             │
│  • confinement mode                                           │
└──────────────────────────────────────────────────────────────┘
```

**No heuristic model scoring is used in policy decisions.**

---

## 6. Zone Model

### Zone Definitions

| Zone | Name | Default Disposition | Description |
|------|------|---------------------|-------------|
| 0 | Workspace | Allow | Agent working directory |
| 1 | UserData | Allow (with safeguards) | User documents, config |
| 2 | Shared | Approval Required | Shared resources, staged exports |
| 3 | SystemCritical | Deny | System files, binaries |
| 4 | Secrets | Deny | Credentials, keys, tokens |

### Classification Sources

1. **Explicit path tables/globs** - Policy-defined patterns
2. **Workspace anchor** - Current project workspace
3. **Ownership fallback** - Root-owned paths are stricter (Unix)
4. **Operation context** - Some operations elevate zone classification

### Zone Precedence

```
Secrets > SystemCritical > Shared > Workspace > UserData
```

This enforces the strongest deterministic policy when path patterns overlap.

---

## 7. Linux Confinement Design

### Bubblewrap Configuration

```bash
bwrap \
  --ro-bind / / \                    # Read-only system
  --bind <runtime-root>/workspace <runtime-root>/workspace \      # RW workspace
  --ro-bind <runtime-root>/shared-zone <runtime-root>/shared-zone \ # Shared-zone visibility
  --ro-bind <empty-dir> <runtime-root>/state \                    # Hide runtime state internals
  --dev /dev \                       # Device access
  --proc /proc \                     # Process info
  --unshare-net \                    # Network namespace (effective deny-all mode)
  --die-with-parent \                # Cleanup on parent exit
  --new-session \                    # New session
  --cap-drop ALL \                   # Drop capabilities
  -- <command>
```

Runtime state masking keeps policy files, approvals queue, receipts store, and execution internals out of confined agent context.

### Preflight Interception

| Command/Pattern | Interception | Reason |
|-----------------|--------------|--------|
| `rm -rf /` | Block | System destruction |
| `rm -rf ~` | Block | Home destruction |
| `rm` (many files) | Require approval | Mass delete guard |
| `mv` (to system paths) | Block | System modification |
| `curl`, `wget` | URL extraction + allowlist | Network control |
| `python`, `bash`, `sh` (with script) | Script origin check | Download→exec prevention |
| `curl | bash` pattern | Block | Stream exec prevention |

### Confinement Degradation Handling

If bubblewrap fails (e.g., uid-map/RTM_NEWADDR errors):

- **Strict mode (default)**: Run fails with explicit confinement-unavailable error + receipt
- **Degraded mode** (`allow_degraded_confinement=true`): Run proceeds without bubblewrap, receipts mark degraded fallback

---

## 8. Network Security Model

### Default Deny Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                   Network Request                           │
│  • URL/host extraction from command line                    │
│  • Method detection (GET vs upload-style)                   │
└─────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│              Default Deny Check                             │
│  If network.default_deny = true and no explicit host-allow  │
│  path is configured:                                        │
│    → Kernel network namespace unshares (deny-all)           │
│  Otherwise:                                                 │
│    → Host rules are mediated by deterministic preflight     │
└─────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│              Domain Allowlist Check                         │
│  • Extract host from URL                                    │
│  • Check against rules.network.allowlist_hosts              │
│  • If not allowlisted → DenyNetworkNotAllowlisted           │
└─────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│              Upload Detection Check                         │
│  • POST/PUT with body                                       │
│  • Form upload patterns                                     │
│  • If upload detected + require_approval_for_post → Approval│
└─────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│              Reserved (Future)                              │
│  • Download byte-size denial reason exists in model         │
│  • Byte-size enforcement not wired in v0.1.7                │
└─────────────────────────────────────────────────────────────┘
```

### URL Query Parameter Exfiltration Detection

URLs are inspected for suspicious patterns:
- Long query parameters (potential data exfil)
- Base64-like content in URLs
- Known exfil endpoint patterns

Detection escalates to approval requirement, not silent block.

---

## 9. Import / Stage / Deliver Workflow

### Three-Phase Flow

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│    IMPORT    │     │    STAGE     │     │   DELIVER    │
│              │     │              │     │              │
│ External     │ ──► │ Workspace    │ ──► │ Shared-Zone  │ ──► │ User Dest    │
│ Source       │     │              │     │              │     │              │
└──────────────┘     └──────────────┘     └──────────────┘     └──────────────┘
      │                    │                    │                    │
      ▼                    ▼                    ▼                    ▼
 Policy Check         Policy Check         Policy Check         Policy Check
 + Approval           + Approval           + Approval           + Approval
 (if needed)          (if needed)          (if needed)          (if needed)
```

### State Tracking

Staged exports transition through states:
1. `PendingStageApproval` - Awaiting approval to stage
2. `Staged` - Copied to shared-zone
3. `PendingDeliveryApproval` - Awaiting approval to deliver
4. `Delivered` - Copied/moved to user destination
5. `Failed` - Operation failed

### Diff Preview

All phases support diff preview:
- Files added/removed/changed
- Bytes added/removed
- Content preview for text files

---

## 10. Safety Pipeline

### Event-Driven Checks

```
┌─────────────────────────────────────────────────────────────┐
│                    Safety Pipeline                          │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
│  │   Normal    │    │   Flagged   │    │  Quarantine │     │
│  │   Operation │───►│   Pattern   │───►│   Escalate  │     │
│  └─────────────┘    └─────────────┘    └─────────────┘     │
│        │                  │                   │             │
│        ▼                  ▼                   ▼             │
│   [Allow]          [Require Approval]   [Quarantine +       │
│                                         Kill Tree if        │
│                                         High Confidence]    │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Key Principles

1. **Do NOT reset everything on "flagged"** - Escalate appropriately
2. **Separate persistent state from ephemeral execution state**
3. **Event-driven checks** on exec/export/download/high-risk triggers
4. **Cache by file hash** for efficiency
5. **Escalation ladder**: flag → quarantine/kill tree → reset exec layer only

### Quarantine Behavior

When quarantine is triggered:
1. File moved to `state/quarantine/`
2. Original path recorded
3. Receipt explains why
4. Kill process tree if high-confidence compromise indicator
5. Reset execution layer only (not persistent state)

---

## 11. Approvals / Elevation

### Approval Flow

```
┌─────────────────────────────────────────────────────────────┐
│                    Approval Request                         │
│  • action: full ActionRequest                               │
│  • reason: ReasonCode                                       │
│  • scope_key: deterministic key for this operation          │
│  • note: human-readable context                             │
└─────────────────────────────────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                    Approval Queue                           │
│  • Stored in approvals.json                                 │
│  • TTL-based expiration                                     │
│  • Status: Pending | Approved | Denied | Expired            │
└─────────────────────────────────────────────────────────────┘
                               │
              ┌────────────────┴────────────────┐
              ▼                                 ▼
┌─────────────────────────┐     ┌─────────────────────────┐
│        Approve          │     │          Deny           │
│  • Update status        │     │  • Update status        │
│  • Apply effect         │     │  • Log receipt          │
│    (copy file, etc.)    │     │  • No action taken      │
│  • Log receipt          │     │                         │
└─────────────────────────┘     └─────────────────────────┘
```

### Elevation Policy

- Critical operations require manual approval
- OS-native elevation only when truly needed
- Never spam elevation prompts
- Bulk approval available with confirmation
- Arbitrary `sudo` passthrough is denied.
- Supported v0.1.7 mediated verb: `install_packages` (`sudo apt install ...` / `sudo apt-get install ...`).
- Elevation requests are converted to deterministic approval records (`approval_required_elevation`).
- On approval, a narrow elevated helper executes fixed args only (no shell), with:
  - allowlisted package checks,
  - max package-count guard,
  - one-time nonce replay protection,
  - explicit receipts for success/failure.

---

## 12. Audit / Receipts

### Receipt Structure

```json
{
  "id": "uuid",
  "timestamp": "2026-02-19T12:00:00Z",
  "action": {
    "id": "uuid",
    "timestamp": "2026-02-19T12:00:00Z",
    "kind": "file_write",
    "operation": "write",
    "path": "/workspace/output.txt",
    "metadata": {},
    "process": {
      "pid": 12345,
      "ppid": 12344,
      "command": "agent-process",
      "process_tree": [12345, 12344, 12340]
    }
  },
  "decision": {
    "verdict": "allow",
    "reason": "allowed_by_policy",
    "detail": "filesystem action allowed in zone Workspace",
    "approval_ttl_seconds": null
  },
  "zone": "workspace",
  "policy_version": "1.0.0",
  "policy_hash": "abc123",
  "diff_summary": {
    "files_added": 1,
    "files_removed": 0,
    "files_changed": 0,
    "bytes_added": 1024,
    "bytes_removed": 0
  },
  "confinement": "bubblewrap"
}
```

### Receipt Guarantees

- Every governed action yields a receipt
- Append-only JSONL format
- Deterministic "why blocked" explanations
- Policy hash/version for reproducibility

---

## 13. Web UI Architecture

### Design System

- Modern admin console aesthetic
- Left navigation + top status bar
- Mobile sidebar toggle + overlay drawer for phone operators
- Consistent components: cards, badges, toasts, modals
- Responsive layout
- No high-frequency polling (event-driven or low-rate)

### Pages

| Page | Purpose |
|------|---------|
| Overview | System snapshot, quick actions |
| Approvals | Pending approvals with bulk actions |
| Files | Import/export/deliver flows |
| Policy | Profile selection, toggles |
| Receipts | Timeline with filters, search, pagination |
| Runtime | Path visibility + editable shared/delivery path settings |
| Execution | Reset execution layer |
| Docs | Integrated documentation |

### API Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/status` | GET | System status |
| `/api/status/feed` | GET | Redacted approval status feed for agent-safe polling |
| `/api/capabilities` | GET | Agent-safe capabilities contract (API version, safe endpoints, redaction guarantees) |
| `/api/runtime` | GET | Runtime paths |
| `/api/runtime/paths` | POST | Update shared-zone and default delivery paths |
| `/api/approvals` | GET | List pending approvals |
| `/api/approvals/:id` | GET | Get single approval (deep-link detail) |
| `/api/approvals/:id/wait` | GET | Long-poll for resolution (`timeout`, `poll_ms`; timeout defaults to runtime Control Panel setting) |
| `/api/approvals/:id/approve` | POST | Approve one |
| `/api/approvals/:id/deny` | POST | Deny one |
| `/api/approvals/approve-all` | POST | Bulk approve |
| `/api/approvals/deny-all` | POST | Bulk deny |
| `/api/receipts` | GET | Paginated receipts |
| `/api/files/list` | GET | List files in zone |
| `/api/export/preview` | POST | Preview stage |
| `/api/export/request` | POST | Request stage |
| `/api/export/deliver/preview` | POST | Preview deliver |
| `/api/export/deliver/request` | POST | Request deliver |
| `/api/import/preview` | POST | Preview import |
| `/api/import/request` | POST | Request import |
| `/api/policy` | GET | Current policy |
| `/api/policy/profiles` | GET | Available profiles |
| `/api/policy/toggles` | POST | Operator policy updates with profile-based locks and baseline safety guards |
| `/api/reset-exec` | POST | Reset execution layer |

**Note:** `/api/policy/toggles` is operator-only. Confined agents use the agent-safe surface (`/api/status/feed`, `/api/approvals/:id/wait`, transfer requests, and tool preflight) and cannot use policy mutation endpoints.

---

## 14. Known v0.1.7 Limitations

| Limitation | Impact | Mitigation |
|------------|--------|------------|
| Enforcement applies to Agent Ruler-launched flows only | Processes outside our runner are not governed | Documentation, safe defaults |
| Linux-first; Windows runner planned | No Windows support yet | Architecture is cross-platform ready |
| No kernel driver/eBPF enforcement | Syscall-level bypass possible | Multiple independent controls, confinement |
| No memory-based exfiltration prevention | Shared memory, pipes not monitored | Network/filesystem focused controls |
| No timing/side-channel prevention | Information leakage possible | Different threat category |

---

## 15. Future Roadmap

### v0.2 (Planned)

- Windows runner with comparable confinement
- eBPF-based syscall monitoring (optional)
- Enhanced exfiltration detection
- Agent adapters for improved telemetry

### v0.3+ (Exploratory)

- Distributed policy management
- Multi-agent orchestration
- Cloud deployment modes
- Formal verification of policy engine

---

## 16. Module Map and Codebase Navigation

This section provides a guided tour of the source code structure, helping developers understand where different concerns live and how modules interact.

### Source Tree Overview

```
src/
├── lib.rs                    # Crate root with module declarations and crate-level docs
├── main.rs                   # CLI entry point (thin wrapper)
├── model.rs                  # Core domain types: Zone, ActionKind, Decision, ReasonCode
├── config.rs                 # Configuration structures and runtime path resolution
├── approvals.rs              # Approval queue management (pending/approved/denied with TTL)
├── receipts.rs               # Append-only JSONL audit log
├── export_gate.rs            # Import/stage/deliver transfer gates
├── staged_exports.rs         # Export state tracking
├── ui.rs                     # Web UI HTTP server and routing
├── agent_ruler.rs            # High-level orchestration API
├── openclaw_bridge.rs        # OpenClaw bridge config model + generated config helpers
├── utils.rs                  # Shared utilities
│
├── cli/                      # CLI command implementations
│   ├── mod.rs
│   ├── approvals.rs          # `agent-ruler approve/deny` commands
│   ├── smoke.rs              # `agent-ruler smoke` integration tests
│   ├── transfer.rs           # `agent-ruler import/export/deliver` commands
│   └── wait.rs               # `agent-ruler wait` for approval resolution
│
├── helpers/                  # Cross-cutting helper modules
│   ├── mod.rs
│   ├── commands/             # Command helper modules
│   │   ├── mod.rs
│   │   └── ui.rs
│   ├── approvals/            # Approval effects, status, views
│   │   ├── mod.rs
│   │   ├── effects.rs        # Apply approval/denial effects
│   │   ├── status.rs         # Status transitions
│   │   ├── utils.rs          # Approval utilities
│   │   └── views.rs          # Approval formatting
│   ├── policy/               # Policy profile helpers
│   │   ├── mod.rs
│   │   └── profiles.rs       # Built-in policy profiles
│   ├── runtime/              # Runtime path management
│   │   ├── mod.rs
│   │   └── paths.rs          # XDG-aligned path resolution
│   ├── transfer/             # Transfer operation helpers
│   │   ├── mod.rs
│   │   └── actions.rs        # Import/export/deliver actions
│   └── ui/                   # UI API handlers
│       ├── mod.rs
│       ├── openclaw_tool_preflight.rs # OpenClaw tool preflight guard
│       ├── pages.rs          # Page data assembly
│       ├── payloads.rs       # Request/response types
│       ├── runtime_api.rs    # Runtime configuration API
│       └── transfer_api.rs   # File transfer API
│
├── policy/                   # Deterministic policy engine
│   ├── mod.rs                # PolicyEngine struct and evaluate()
│   ├── evaluation.rs         # Zone classification and rule evaluation
│   ├── helpers.rs            # Policy evaluation helpers
│   ├── patterns.rs           # Glob patterns for zone classification
│   └── zone.rs               # Zone definitions and precedence
│
├── runners/                  # Managed runner lifecycle adapters
│   ├── mod.rs
│   └── openclaw.rs
│
└── runner/                   # Command execution and confinement
    ├── mod.rs                # run_confined() orchestration + RunResult
    ├── preflight.rs          # Pre-execution interception (rm, curl, etc.)
    └── confinement/          # Platform-specific sandboxing
        ├── mod.rs            # Platform detection and module re-exports
        ├── common.rs         # Shared types (ConfinementBackend) and unconfined fallback
        ├── linux.rs          # Bubblewrap (bwrap) implementation
        ├── windows.rs        # Windows stub (future)
        └── macos.rs          # macOS stub (future)
```

### Module Boundaries and Responsibilities

| Module | Responsibility | Key Types |
|--------|---------------|-----------|
| `model.rs` | Core domain model | `Zone`, `ActionKind`, `ActionRequest`, `Decision`, `ReasonCode` |
| `config.rs` | Runtime config and path resolution | `AppConfig`, `Policy`, `RuntimeState`, `RuntimeLayout` |
| `policy/mod.rs` | Deterministic policy engine | `PolicyEngine` |
| `policy/evaluation.rs` | Zone classification and rule evaluation | deterministic zone/rule evaluators |
| `runner/mod.rs` | Command execution orchestration | `run_confined()`, `RunResult` |
| `runner/preflight.rs` | Command interception and gating | utility/network/persistence/interpreter preflight helpers |
| `runner/confinement/` | Platform sandboxing | `build_confined_command()`, `ConfinementBackend` |
| `approvals.rs` | Approval queue | `ApprovalStore`, `ApprovalRecord`, `ApprovalStatus` |
| `receipts.rs` | Audit logging | `ReceiptStore`, `Receipt` |
| `ui.rs` | HTTP server | UI endpoints and routing |
| `helpers/ui/` | UI API handlers | Request/response handling |
| `runners/openclaw.rs` | Managed OpenClaw lifecycle and config guards | `OpenClawAdapter` |
| `openclaw_bridge.rs` | OpenClaw bridge config normalization | `OpenClawBridgeConfig` |

### Key Data Flows

1. **Action Evaluation Flow**:
   ```
   ActionRequest → PolicyEngine::evaluate() → policy/evaluation.rs → Decision
                                                              ↓
                                             Zone classification (policy/zone.rs)
                                                              ↓
                                             Rule application (policy/evaluation.rs)
   ```

2. **Command Execution Flow**:
   ```
   CLI/Agent → run_confined() → runner/preflight.rs (interception)
                              ↓
                              runner/confinement/ (sandboxing)
                              ↓
                              RunResult
   ```

3. **Approval Flow**:
   ```
   Decision::RequireApproval → approvals.rs (queue) → User approval
                                                          ↓
                                          helpers/approvals/effects.rs (apply)
   ```

### Platform-Specific Code

The `runner/confinement/` directory uses conditional compilation for platform support:

- **Linux** (`linux.rs`): Full bubblewrap implementation with namespace isolation
- **Windows** (`windows.rs`): Stub for future implementation
- **macOS** (`macos.rs`): Stub for future implementation
- **Common** (`common.rs`): Shared `ConfinementBackend` enum and `run_unconfined()` fallback

Platform detection happens at compile time via `#[cfg(target_os = "...")]` attributes.

### Security Invariants

The following invariants are documented in code comments and must be preserved:

1. **Determinism**: `PolicyEngine::evaluate()` must return the same decision for the same input
2. **Zone Precedence**: Secrets > SystemCritical > Shared > Workspace > UserData
3. **Receipt Completeness**: Every governed action yields a receipt
4. **Confinement Default**: Processes run confined unless explicitly degraded
5. **Approval Scoping**: Approvals are scope-keyed for targeted permissions
