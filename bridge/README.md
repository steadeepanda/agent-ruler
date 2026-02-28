# Bridge Layout

Bridge assets are now runner-scoped so future runners can ship isolated adapters without mixing files.

## Current runner namespace

- `bridge/openclaw/channel_bridge.py` - OpenClaw approval channel bridge runtime
- `bridge/openclaw/approvals-hook/` - inbound hook pack (`agent-ruler-approvals`)
- `bridge/openclaw/openclaw-agent-ruler-tools/` - OpenClaw tools adapter plugin
- `bridge/openclaw/samples/` - inbound payload samples
- `bridge/openclaw/smoke_replay.py` - local bridge smoke replay
- `bridge/openclaw/tests/` - bridge parser/runtime tests

## Backward compatibility

- `bridge/openclaw_channel_bridge.py` remains as a compatibility shim and forwards to `bridge/openclaw/channel_bridge.py`.
- Legacy route files (`openclaw-channel-bridge.json`, `openclaw-channel-bridge.local.json`) are still detected for migration.

## Typical OpenClaw setup command snippets

```bash
agent-ruler run -- openclaw config set plugins.load.paths "[\"$AR_DIR/bridge/openclaw/openclaw-agent-ruler-tools\"]" --json
agent-ruler run -- openclaw hooks install "$AR_DIR/bridge/openclaw/approvals-hook"
python3 "$AR_DIR/bridge/openclaw/channel_bridge.py" --config "$RUNTIME_ROOT/user_data/bridge/openclaw-channel-bridge.generated.json"
```
