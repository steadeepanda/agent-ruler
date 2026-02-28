# Demo Scripts

Run from project root after building the binary.

```bash
chmod +x demo/*.sh
./demo/01-normal-workspace.sh
./demo/02-block-system-delete.sh
./demo/03-download-exec-guard.sh
./demo/04-export-approval.sh
```

Note:
- each script runs `agent-ruler init --force`, so runtime state is reset per script.
- runtime data is created outside the repo by default (under the user runtime root).

Optional: pass binary path as first argument.

```bash
./demo/01-normal-workspace.sh ./target/release/agent-ruler
```
