import { readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { defineConfig } from 'vitepress';

const __dirname = dirname(fileURLToPath(import.meta.url));

// Read version from Cargo.toml
const cargoTomlPath = resolve(__dirname, '../../../Cargo.toml');
const cargoToml = readFileSync(cargoTomlPath, 'utf8');
const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
const version = versionMatch ? versionMatch[1] : 'unknown';

type ManifestItem = {
  title: string;
  route: string;
  source?: string;
  generated?: boolean;
};

type ManifestSection = {
  id: string;
  title: string;
  items: ManifestItem[];
};

type DocsManifest = {
  sections: ManifestSection[];
};

const manifestPath = resolve(__dirname, 'docs-manifest.json');
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as DocsManifest;

const sectionOrder = [
  'getting-started',
  'integrations',
  'concepts',
  'security',
  'cli-reference',
  'troubleshooting'
];

const sectionMap = new Map(manifest.sections.map((section) => [section.id, section]));
const orderedSections = sectionOrder
  .map((id) => sectionMap.get(id))
  .filter((section): section is ManifestSection => Boolean(section));

const sidebar = orderedSections.map((section) => ({
  text: section.title,
  items: section.items.map((item) => ({ text: item.title, link: `/${item.route}` }))
}));

const nav = [
  { text: 'Getting Started', link: '/guides/getting-started' },
  { text: 'Integrations', link: '/integrations/openclaw-guide' },
  { text: 'Concepts', link: '/concepts/zones-and-flows' },
  { text: 'Security', link: '/security/prompt-injection' },
  { text: 'CLI', link: '/reference/cli' },
  { text: 'Troubleshooting', link: '/troubleshooting/common-issues' }
];

// Base is /help/ because the daemon serves docs at that path.
export default defineConfig({
  title: 'Agent Ruler Docs',
  description: 'Deterministic reference monitor and confinement control panel',
  lang: 'en-US',
  // Axum serves prebuilt static files directly; explicit `.html` links avoid
  // clean-URL rewrite mismatches and stale hydration artifacts.
  cleanUrls: false,
  base: '/help/',
  appearance: true,

  head: [
    [
      'script',
      {},
      `(() => {
        const key = 'vitepress-theme-appearance';
        const stored = localStorage.getItem(key);
        const theme = stored === 'light' ? 'light' : 'dark';
        localStorage.setItem(key, theme);
        document.documentElement.classList.toggle('dark', theme === 'dark');
        document.documentElement.dataset.theme = theme;
      })();`
    ],
    [
      'style',
      {},
      `:root{--ar-docs-version-label:"v${version}";}`
    ]
  ],
  themeConfig: {
    logo: '/agent-ruler-mark.svg',
    siteTitle: 'Agent Ruler',
    nav,
    sidebar,
    search: {
      provider: 'local',
      options: {
        miniSearch: {
          searchOptions: {
            boost: { title: 6, text: 1.5 },
            fuzzy: 0.15
          }
        },
        translations: {
          button: {
            buttonText: 'Search docs',
            buttonAriaLabel: 'Search docs (Ctrl+K)'
          },
          modal: {
            noResultsText: 'No results found',
            resetButtonTitle: 'Clear',
            footer: {
              selectText: 'to open',
              navigateText: 'to navigate',
              closeText: 'to close'
            }
          }
        }
      }
    },
    outline: {
      label: 'On this page',
      level: [2, 3]
    },
    docFooter: {
      prev: 'Previous',
      next: 'Next'
    }
  }
});
