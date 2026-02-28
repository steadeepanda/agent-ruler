import { promises as fs } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..', '..');
const docsRoot = path.resolve(__dirname, '..', 'docs');
const manifestPath = path.join(docsRoot, '.vitepress', 'docs-manifest.json');

async function ensureDir(dirPath) {
  await fs.mkdir(dirPath, { recursive: true });
}

function normalizeContent(content) {
  if (content.startsWith('\uFEFF')) {
    return content.slice(1);
  }
  return content;
}

async function syncGeneratedPages(manifest) {
  for (const section of manifest.sections || []) {
    for (const item of section.items || []) {
      if (!item.generated) continue;
      if (!item.source || !item.route) continue;

      const sourcePath = path.resolve(repoRoot, item.source);
      const outputPath = path.resolve(docsRoot, `${item.route}.md`);

      const raw = await fs.readFile(sourcePath, 'utf8');
      const content = normalizeContent(raw);
      const generatedHeader = [
        `---`,
        `title: ${item.title}`,
        `---`,
        ``,
        `> Synced automatically from \`${item.source}\`. Edit the source file and run \`npm --prefix docs-site run docs:sync\`.`,
        ``
      ].join('\n');

      await ensureDir(path.dirname(outputPath));
      await fs.writeFile(outputPath, `${generatedHeader}${content}`);
    }
  }
}

async function syncSharedAssets() {
  const sourceTokens = path.resolve(repoRoot, 'assets', 'design-tokens.css');
  const sourceLogo = path.resolve(repoRoot, 'assets', 'logo-mark.svg');
  const publicDir = path.resolve(docsRoot, 'public');
  await ensureDir(publicDir);
  await fs.copyFile(sourceTokens, path.join(publicDir, 'design-tokens.css'));
  await fs.copyFile(sourceLogo, path.join(publicDir, 'agent-ruler-mark.svg'));
}

async function main() {
  const manifestRaw = await fs.readFile(manifestPath, 'utf8');
  const manifest = JSON.parse(manifestRaw);

  await syncGeneratedPages(manifest);
  await syncSharedAssets();

  console.log('Synced curated generated docs and shared design assets.');
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
