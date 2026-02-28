import DefaultTheme from 'vitepress/theme';

import './custom.css';

const THEME_KEY = 'vitepress-theme-appearance';

function syncThemeDataset() {
  const root = document.documentElement;
  const dark = root.classList.contains('dark');
  root.dataset.theme = dark ? 'dark' : 'light';
  localStorage.setItem(THEME_KEY, dark ? 'dark' : 'light');
}

export default {
  ...DefaultTheme,
  enhanceApp() {
    if (typeof window === 'undefined') return;

    syncThemeDataset();

    const observer = new MutationObserver(() => {
      syncThemeDataset();
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class']
    });
  }
};
