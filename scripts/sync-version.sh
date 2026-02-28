#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

read_cargo_version() {
  sed -nE 's/^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"/\1/p' "$ROOT_DIR/Cargo.toml" | head -n1
}

VERSION="${1:-$(read_cargo_version)}"
VERSION="${VERSION#v}"
if [[ -z "$VERSION" ]]; then
  echo "Could not resolve version. Pass one explicitly, e.g. scripts/sync-version.sh 0.1.1" >&2
  exit 1
fi

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Version must use semver core format X.Y.Z (got: $VERSION)" >&2
  exit 1
fi

CARGO_VERSION="$(read_cargo_version)"
if [[ "$CARGO_VERSION" != "$VERSION" ]]; then
  perl -0pi -e 's/^version\s*=\s*"[0-9]+\.[0-9]+\.[0-9]+"/version = "'"$VERSION"'"/m' "$ROOT_DIR/Cargo.toml"
fi

export ROOT_DIR VERSION
node <<'NODE'
const fs = require('fs');
const path = require('path');

const root = process.env.ROOT_DIR;
const version = process.env.VERSION;

function updateJson(relPath, mutator) {
  const abs = path.join(root, relPath);
  const data = JSON.parse(fs.readFileSync(abs, 'utf8'));
  mutator(data);
  fs.writeFileSync(abs, JSON.stringify(data, null, 2) + '\n');
}

updateJson('docs-site/package.json', (data) => {
  data.version = version;
});

updateJson('docs-site/package-lock.json', (data) => {
  data.version = version;
  if (data.packages && data.packages['']) {
    data.packages[''].version = version;
  }
});

updateJson('bridge/openclaw/openclaw-agent-ruler-tools/openclaw.plugin.json', (data) => {
  data.version = version;
});
NODE

perl -0pi -e 's/(^version:\s*)[0-9]+\.[0-9]+\.[0-9]+/${1}'"$VERSION"'/m' "$ROOT_DIR/bridge/openclaw/approvals-hook/HOOK.md"
for file in \
  "$ROOT_DIR/SECURITY.md" \
  "$ROOT_DIR/docs/architecture.md" \
  "$ROOT_DIR/docs/security/prompt-injection.md" \
  "$ROOT_DIR/docs-site/docs/concepts/architecture.md" \
  "$ROOT_DIR/docs-site/docs/security/prompt-injection.md" \
  "$ROOT_DIR/docs-site/docs/security/security-policy.md"; do
  [[ -f "$file" ]] || continue
  perl -0pi -e 's/v[0-9]+\.[0-9]+\.[0-9]+(?:\.[0-9]+)?/v'"$VERSION"'/g;' "$file"
done

echo "Version sync complete: $VERSION"
