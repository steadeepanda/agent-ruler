#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT_DIR

read_app_config() {
  node <<'NODE'
const fs = require('fs');
const path = require('path');

const root = process.env.ROOT_DIR;
const manifestPath = path.join(root, 'config', 'app.json');
const data = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
const version = String(data.version ?? '').trim();
const tag = String(data.tag ?? `v${version}`).trim();
const publicRepo = String(data.public_repo ?? '').trim();

if (!version) {
  throw new Error('config/app.json is missing a version field');
}

if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`config/app.json version must be a semver release or prerelease (got: ${version})`);
}

if (tag !== `v${version}`) {
  throw new Error(`config/app.json tag must match the version field (expected v${version}, got: ${tag})`);
}

if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(publicRepo)) {
  throw new Error(`config/app.json public_repo must look like owner/repo (got: ${publicRepo || '<empty>'})`);
}

process.stdout.write(`${version}\n${tag}\n${publicRepo}\n`);
NODE
}

readarray -t VERSION_LINES < <(read_app_config)
VERSION="${VERSION_LINES[0]}"
TAG="${VERSION_LINES[1]}"
PUBLIC_REPO="${VERSION_LINES[2]}"

perl -0pi -e 's/^version\s*=\s*"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?"/version = "'"$VERSION"'"/m' "$ROOT_DIR/Cargo.toml"

export ROOT_DIR VERSION PUBLIC_REPO
node <<'NODE'
const fs = require('fs');
const path = require('path');

const root = process.env.ROOT_DIR;
const version = process.env.VERSION;
const publicRepo = process.env.PUBLIC_REPO;

function updateJson(relPath, mutator) {
  const abs = path.join(root, relPath);
  const data = JSON.parse(fs.readFileSync(abs, 'utf8'));
  mutator(data);
  fs.writeFileSync(abs, JSON.stringify(data, null, 2) + '\n');
}

function updateText(relPath, replacer) {
  const abs = path.join(root, relPath);
  const raw = fs.readFileSync(abs, 'utf8');
  const next = replacer(raw);
  fs.writeFileSync(abs, next);
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

updateText('src/cli/update.rs', (raw) =>
  raw.replace(
    /const DEFAULT_GITHUB_REPO: &str = "[^"]+";/,
    `const DEFAULT_GITHUB_REPO: &str = "${publicRepo}";`
  )
);

updateText('install/install.sh', (raw) =>
  raw.replace(
    /(# default for normal users \(no repo checkout\)\n\s*printf '%s' )"[^"]+"/,
    `$1"${publicRepo}"`
  )
);
NODE

perl -0pi -e 's/(^version:\s*)[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?/${1}'"$VERSION"'/m' "$ROOT_DIR/bridge/openclaw/approvals-hook/HOOK.md"
perl -0pi -e 's/(This recording shows the redesigned Control Panel flow for `)v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(`\.)/${1}v'"$VERSION"'${2}/' "$ROOT_DIR/README.md"
perl -0pi -e 's/(This recording shows the redesigned Control Panel flow for `)v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(`\.)/${1}v'"$VERSION"'${2}/' "$ROOT_DIR/docs-site/docs/guides/control-panel.md"
for file in \
  "$ROOT_DIR/SECURITY.md" \
  "$ROOT_DIR/docs-site/docs/concepts/bridge-architecture.md" \
  "$ROOT_DIR/docs-site/docs/concepts/architecture.md" \
  "$ROOT_DIR/docs-site/docs/security/prompt-injection.md" \
  "$ROOT_DIR/docs-site/docs/security/security-policy.md"; do
  [[ -f "$file" ]] || continue
  perl -0pi -e 's/v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?/v'"$VERSION"'/g;' "$file"
done

echo "Version sync complete: $VERSION"
