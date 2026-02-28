(function (factory) {
  // UMD-style export so the same helpers work in browser bundle and node tests.
  const globalRoot =
    typeof globalThis !== 'undefined'
      ? globalThis
      : typeof self !== 'undefined'
        ? self
        : typeof window !== 'undefined'
          ? window
          : typeof global !== 'undefined'
            ? global
            : {};

  const exported = factory();
  if (typeof module === 'object' && typeof module.exports === 'object') {
    module.exports = exported;
  }
  globalRoot.pathBrowserUtils = exported;
})(function () {
  // Browser prefixes are always clamped to the current zone root.
  function normalizeBrowserPrefix(value) {
    const segments = String(value || '')
      .replace(/\\/g, '/')
      .split('/')
      .map(segment => segment.trim())
      .filter(segment => segment && segment !== '.');

    const resolved = [];
    segments.forEach((segment) => {
      if (segment === '..') {
        if (resolved.length) {
          resolved.pop();
        }
        return;
      }
      resolved.push(segment);
    });

    return resolved.join('/');
  }

  function segments(prefix) {
    const normalized = normalizeBrowserPrefix(prefix);
    return normalized ? normalized.split('/') : [];
  }

  function joinPaths(parent, child) {
    const normalizedParent = normalizeBrowserPrefix(parent);
    const normalizedChild = normalizeBrowserPrefix(child);
    if (!normalizedParent) return normalizedChild;
    if (!normalizedChild) return normalizedParent;
    return `${normalizedParent}/${normalizedChild}`;
  }

  function basename(path) {
    if (!path) return '';
    const normalized = path.replace(/\\/g, '/').replace(/\/+$/, '');
    const parts = normalized.split('/');
    return parts[parts.length - 1];
  }

  return {
    normalizeBrowserPrefix,
    joinPaths,
    basename,
    segments,
  };
});
