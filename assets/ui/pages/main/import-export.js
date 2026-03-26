  // Extracted from assets/ui/10-pages-main.js for page-scoped editing.
  const zoneBrowserInstances = {
    workspace: null,
    shared: null,
    deliver: null
  };
  const IMPORT_EXPORT_RUNNER_STORAGE_KEY = 'ar.files.runner';
  const IMPORT_EXPORT_RUNNERS = ['openclaw', 'claudecode', 'opencode'];
  let importExportRunnerSelection = null;

  const fallbackNormalizePrefix = (value) => {
    const segments = String(value || '')
      .replace(/\\/g, '/')
      .split('/')
      .map((segment) => segment.trim())
      .filter((segment) => segment && segment !== '.');
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
  };

  const fallbackJoinPaths = (parent, child) => {
    const normalizedParent = fallbackNormalizePrefix(parent);
    const normalizedChild = fallbackNormalizePrefix(child);
    if (!normalizedParent) return normalizedChild;
    if (!normalizedChild) return normalizedParent;
    return `${normalizedParent}/${normalizedChild}`;
  };

  const fallbackBasename = (path) => {
    if (!path) return '';
    const normalized = path.replace(/\\/g, '/').replace(/\/+$/, '');
    const parts = normalized.split('/');
    return parts[parts.length - 1];
  };

  const pathUtils =
    typeof pathBrowserUtils !== 'undefined' && pathBrowserUtils
      ? pathBrowserUtils
      : {
          normalizeBrowserPrefix: fallbackNormalizePrefix,
          joinPaths: fallbackJoinPaths,
          basename: fallbackBasename,
        };

  const normalizePrefix = pathUtils.normalizeBrowserPrefix;
  const basename = pathUtils.basename;

  function normalizeImportExportRunner(value) {
    const normalized = String(value || '').trim().toLowerCase();
    return IMPORT_EXPORT_RUNNERS.includes(normalized) ? normalized : null;
  }

  function preferredImportExportRunner() {
    const fromStorage = normalizeImportExportRunner(localStorage.getItem(IMPORT_EXPORT_RUNNER_STORAGE_KEY));
    if (fromStorage) return fromStorage;

    const fromRuntime = normalizeImportExportRunner(state.runtime?.selected_runner);
    if (fromRuntime) return fromRuntime;

    const fromStatus = normalizeImportExportRunner(state.status?.selected_runner);
    if (fromStatus) return fromStatus;

    return 'openclaw';
  }

  function buildFilesListUrl(zone, prefix, limit = 400, runnerId = null) {
    const params = new URLSearchParams();
    params.set('zone', zone);
    params.set('limit', String(limit));
    const normalizedPrefix = normalizePrefix(prefix || '');
    if (normalizedPrefix) {
      params.set('prefix', normalizedPrefix);
    }
    const normalizedRunner = normalizeImportExportRunner(runnerId);
    if (normalizedRunner) {
      params.set('runner', normalizedRunner);
    }
    return `/api/files/list?${params.toString()}`;
  }

  function createZoneFileBrowser({
    container,
    zoneKind,
    title,
    chipLabel,
    selectionMode = 'multi',
    dropConfig = null,
    runnerProvider = null
  }) {
    if (!container) return null;

    container.classList.add('zone-browser-host');
    container.innerHTML = `
      <div class="zone-browser" data-zone="${zoneKind}" style="display: flex; flex-direction: column; width: 100%; height: 100%; background: var(--content-bg);">
        
        <div class="zone-browser-header" style="padding: var(--space-3); border-bottom: 1px solid var(--content-border); background: var(--content-bg-alt);">
          <div class="zone-browser-header-actions" style="display: flex; justify-content: flex-end; gap: var(--space-2); margin-bottom: var(--space-2);">
            <button type="button" class="btn btn-sm btn-outline" data-action="up"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="margin-right: 4px;"><path d="M12 19V5M5 12l7-7 7 7"/></svg> Up</button>
            <button type="button" class="btn btn-sm btn-outline" data-action="refresh"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="margin-right: 4px;"><path d="M21 2v6h-6"/><path d="M3 12a9 9 0 1 0 2.6-6.4L2 9"/></svg> Refresh</button>
          </div>
          <div class="zone-browser-breadcrumb mono" data-browser-breadcrumb style="display: block; width: 100%; margin: 0; padding: var(--space-2) var(--space-3); border: 1px solid var(--content-border); border-radius: var(--radius); background: var(--content-bg); text-align: left; overflow-x: auto; white-space: nowrap; font-size: 0.85rem; color: var(--text-primary);"></div>
        </div>

        <div class="zone-browser-selection-controls" data-selection-controls style="padding: var(--space-2) var(--space-4); background: var(--hover-bg); border-bottom: 1px solid var(--content-border); display: flex; align-items: center; gap: var(--space-4);">
          <label class="form-check" style="margin: 0;">
            <input type="checkbox" class="form-check-input" data-action="select-all" />
            <span class="form-check-label">Select All</span>
          </label>
          <span class="zone-browser-selection-count" data-selection-count style="color: var(--text-muted); font-size: 0.85rem; margin-right: auto; padding-left: 10px;">0 selected</span>
          <button type="button" class="btn btn-sm btn-outline" data-action="clear-selection">Clear</button>
        </div>

        <div class="zone-browser-status" data-browser-status></div>
        <div class="zone-browser-list" data-browser-list role="list" style="flex: 1; overflow-y: auto; padding: var(--space-2);"></div>
        <div class="zone-browser-drop-hint">Drag items here to ${zoneKind === 'shared' ? 'stage them' : zoneKind === 'deliver' ? 'deliver them' : 'explore'}.</div>
      </div>
    `;

    const root = container.querySelector('.zone-browser');
    const breadcrumbEl = container.querySelector('[data-browser-breadcrumb]');
    const listEl = container.querySelector('[data-browser-list]');
    const statusEl = container.querySelector('[data-browser-status]');
    const selectAllEl = container.querySelector('[data-action="select-all"]');
    const selectionCountEl = container.querySelector('[data-selection-count]');
    const clearSelectionBtn = container.querySelector('[data-action="clear-selection"]');
    const refreshBtn = container.querySelector('[data-action="refresh"]');
    const upBtn = container.querySelector('[data-action="up"]');

    const state = {
      prefix: '',
      entries: [],
      viewEntries: [],
      selection: new Set(),
      lastClickIndex: null,
      loading: false,
      dragDropEnabled: shouldAllowDragDrop()
    };

    const loadingTemplate = '<div class="loading"><div class="spinner"></div></div>';

    const dropHint = dropConfig ? dropConfig.hint : null;
    if (dropHint) {
      const hintEl = container.querySelector('.zone-browser-drop-hint');
      if (hintEl) hintEl.textContent = dropHint;
    } else {
      const hintEl = container.querySelector('.zone-browser-drop-hint');
      if (hintEl) hintEl.textContent = 'Browse the zone by double-clicking folders and checkboxes.';
    }

    function setStatus(message, level = 'info') {
      if (!statusEl) return;
      statusEl.textContent = message;
      statusEl.className = `zone-browser-status zone-browser-status-${level}`;
    }

    function clearStatus() {
      if (statusEl) {
        statusEl.textContent = '';
        statusEl.className = 'zone-browser-status';
      }
    }

    function updateBreadcrumb() {
      if (!breadcrumbEl) return;
      const prefix = state.prefix ? state.prefix : '';
      const segments = prefix ? prefix.split('/') : [];
      const crumbs = ['<button type="button" class="zone-breadcrumb" data-breadcrumb="">Root</button>'];
      let acc = '';
      segments.forEach((seg, idx) => {
        acc = idx === 0 ? seg : `${acc}/${seg}`;
        crumbs.push(`<span class="zone-breadcrumb-sep">/</span><button type="button" class="zone-breadcrumb" data-breadcrumb="${acc}">${esc(seg)}</button>`);
      });
      breadcrumbEl.innerHTML = `<div class="zone-browser-breadcrumb-inner">${crumbs.join('')}</div>`;
      breadcrumbEl.querySelectorAll('[data-breadcrumb]').forEach((btn) => {
        btn.addEventListener('click', () => {
          const target = btn.dataset.breadcrumb || '';
          loadEntries(target);
        });
      });
    }

    function buildViewEntries(prefix, entries) {
      const normalizedPrefix = normalizePrefix(prefix || '');
      const bucket = new Map();
      entries.forEach((item) => {
        if (normalizedPrefix && !item.path.startsWith(`${normalizedPrefix}/`)) {
          return;
        }
        const rel = normalizedPrefix ? item.path.slice(normalizedPrefix.length + 1) : item.path;
        if (!rel) return;
        const idx = rel.indexOf('/');
        const name = idx === -1 ? rel : rel.slice(0, idx);
        const childPath = normalizedPrefix ? `${normalizedPrefix}/${name}` : name;
        const isDir = idx !== -1 || item.kind === 'dir';
        const existing = bucket.get(childPath);
        if (!existing || (existing.kind === 'file' && isDir)) {
          bucket.set(childPath, {
            name,
            path: childPath,
            kind: isDir ? 'dir' : 'file',
            bytes: isDir ? 0 : item.bytes || 0
          });
        }
      });
      return Array.from(bucket.values()).sort((a, b) => {
        if (a.kind === b.kind) return a.name.localeCompare(b.name);
        return a.kind === 'dir' ? -1 : 1;
      });
    }

    function renderEntries() {
      if (!listEl) return;
      const rows = [];
      if (state.prefix) {
        rows.push(`
          <div class="zone-browser-item zone-browser-item-up" data-action="up-row">
            <div class="zone-browser-item-main">
              <span class="zone-browser-item-icon">↥</span>
              <div>
                <div class="zone-browser-item-name">.. (parent)</div>
                <div class="zone-browser-item-meta">Go up one level</div>
              </div>
            </div>
          </div>
        `);
      }
      if (!state.viewEntries.length) {
        rows.push('<div class="zone-browser-empty">No files in this folder.</div>');
      } else {
        state.viewEntries.forEach((entry, idx) => {
          const icon = entry.kind === 'dir' ? '📁' : '📄';
          const meta = entry.kind === 'dir' ? 'Directory' : formatBytes(entry.bytes);
          const checkbox = selectionMode === 'multi' ? `
            <label class="form-check zone-browser-check">
              <input type="checkbox" class="form-check-input zone-browser-checkbox" data-path="${esc(entry.path)}" ${state.selection.has(entry.path) ? 'checked' : ''} />
              <span class="form-check-label">Select</span>
            </label>
          ` : '';
          rows.push(`
            <div class="zone-browser-item ${entry.kind === 'dir' ? 'zone-browser-directory' : 'zone-browser-file'}" data-index="${idx}" data-path="${esc(entry.path)}" data-kind="${entry.kind}" draggable="${state.dragDropEnabled ? 'true' : 'false'}">
              <div class="zone-browser-item-main">
                <span class="zone-browser-item-icon">${icon}</span>
                <div>
                  <div class="zone-browser-item-name">${esc(entry.name)}</div>
                  <div class="zone-browser-item-meta">${meta}</div>
                </div>
              </div>
              <div class="zone-browser-item-actions">${checkbox}</div>
            </div>
          `);
        });
      }
      listEl.innerHTML = rows.join('');
      if (state.viewEntries.length) attachRowListeners();
      updateSelectionControls();
    }

    function attachRowListeners() {
      if (!listEl) return;
      listEl.querySelectorAll('.zone-browser-item').forEach((item) => {
        const index = Number(item.dataset.index);
        if (!Number.isFinite(index)) {
          if (item.dataset.action === 'up-row') {
            item.addEventListener('click', (event) => {
              event.preventDefault();
              goUp();
            });
            item.addEventListener('dblclick', (event) => {
              event.preventDefault();
              goUp();
            });
          }
          return;
        }
        const path = item.dataset.path;
        const kind = item.dataset.kind;
        const checkbox = item.querySelector('.zone-browser-checkbox');

        item.addEventListener('dblclick', (event) => {
          event.preventDefault();
          if (kind === 'dir') {
            loadEntries(path);
            return;
          }
          toggleSelection(path, index, { shiftKey: event.shiftKey });
        });

        item.addEventListener('click', (event) => {
          if (event.target.closest('.zone-browser-checkbox')) return;
          toggleSelection(path, index, { shiftKey: event.shiftKey, ctrlKey: event.ctrlKey || event.metaKey });
        });

        if (checkbox) {
          checkbox.addEventListener('change', () => {
            toggleSelection(path, index, { selected: checkbox.checked });
          });
        }

        item.addEventListener('dragstart', (event) => {
          if (!state.dragDropEnabled) {
            event.preventDefault();
            return;
          }
          const currentSelection = Array.from(state.selection);
          const dragging = currentSelection.length && currentSelection.includes(path) ? currentSelection : [path];
          const payload = JSON.stringify({ zone: zoneKind, paths: dragging });
          event.dataTransfer.effectAllowed = 'copy';
          event.dataTransfer.setData('application/json', payload);
          event.dataTransfer.setData('text/plain', dragging.join(', '));
        });
      });
    }

    function toggleSelection(path, index, options = {}) {
      if (selectionMode !== 'multi') {
        state.selection.clear();
        state.selection.add(path);
        state.lastClickIndex = index;
        updateSelectionControls();
        return;
      }

      const { shiftKey, ctrlKey, selected } = options;
      if (typeof selected === 'boolean') {
        if (selected) state.selection.add(path);
        else state.selection.delete(path);
      } else if (shiftKey && state.lastClickIndex !== null) {
        const [start, end] = [state.lastClickIndex, index].sort((a, b) => a - b);
        for (let cursor = start; cursor <= end; cursor += 1) {
          const entry = state.viewEntries[cursor];
          if (entry) state.selection.add(entry.path);
        }
      } else {
        if (state.selection.has(path)) {
          state.selection.delete(path);
        } else {
          state.selection.add(path);
        }
      }
      state.lastClickIndex = index;
      updateSelectionControls();
    }

    function updateSelectionControls() {
      if (!selectionCountEl) return;
      const count = state.selection.size;
      selectionCountEl.textContent = `${count} selected`;
      if (selectAllEl) {
        const total = state.viewEntries.length;
        selectAllEl.checked = total > 0 && count === total;
        selectAllEl.indeterminate = count > 0 && count < total;
      }
    }

    function clearSelection() {
      state.selection.clear();
      state.lastClickIndex = null;
      if (listEl) {
        listEl.querySelectorAll('.zone-browser-checkbox').forEach((cb) => { cb.checked = false; });
      }
      updateSelectionControls();
    }

    if (selectAllEl) {
      selectAllEl.addEventListener('change', () => {
        if (selectAllEl.checked) {
          state.viewEntries.forEach((entry) => state.selection.add(entry.path));
        } else {
          state.selection.clear();
        }
        updateSelectionControls();
        if (listEl) {
          listEl.querySelectorAll('.zone-browser-checkbox').forEach((cb) => { cb.checked = selectAllEl.checked; });
        }
      });
    }

    if (clearSelectionBtn) {
      clearSelectionBtn.addEventListener('click', clearSelection);
    }

    if (refreshBtn) {
      refreshBtn.addEventListener('click', () => loadEntries(state.prefix));
    }

    if (upBtn) {
      upBtn.addEventListener('click', goUp);
    }

    if (dropConfig && listEl) {
      listEl.addEventListener('dragover', (event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = 'copy';
        root.classList.add('zone-browser-drop-target');
      });
      listEl.addEventListener('dragleave', () => {
        root.classList.remove('zone-browser-drop-target');
      });
      listEl.addEventListener('drop', async (event) => {
        event.preventDefault();
        root.classList.remove('zone-browser-drop-target');
        const payload = event.dataTransfer.getData('application/json');
        if (!payload) return;
        let data;
        try {
          data = JSON.parse(payload);
        } catch (err) {
          return;
        }
        if (!data.paths || !Array.isArray(data.paths) || data.zone !== dropConfig.from) {
          setStatus(dropConfig.invalidMessage, 'warning');
          return;
        }
        if (!shouldAllowDragDrop()) {
          setStatus('Drag & drop requires Action Source to be User', 'warning');
          return;
        }
        setStatus('Processing drop...', 'info');
        try {
          await dropConfig.handler(data.paths, state.prefix || '');
        } catch (err) {
          toast(`Drop operation failed: ${err.message || err}`, 'error');
        } finally {
          clearStatus();
        }
      });
    }

    async function goUp() {
      if (!state.prefix) return;
      const parts = state.prefix.split('/');
      parts.pop();
      await loadEntries(parts.join('/'));
    }

    async function loadEntries(prefixOverride = '') {
      const prefix = normalizePrefix(prefixOverride);
      state.prefix = prefix;
      updateBreadcrumb();
      if (!listEl) return;
      listEl.innerHTML = loadingTemplate;
      state.selection.clear();
      state.lastClickIndex = null;
      try {
        const selectedRunner = typeof runnerProvider === 'function' ? runnerProvider() : null;
        const entries = await api(buildFilesListUrl(zoneKind, prefix, 800, selectedRunner));
        state.entries = entries;
        state.viewEntries = buildViewEntries(prefix, entries);
        renderEntries();
        clearStatus();
      } catch (err) {
        setStatus(`Failed to load ${title.toLowerCase()} contents: ${err.message}`, 'error');
        listEl.innerHTML = '<div class="zone-browser-error">Failed to load contents</div>';
      }
    }

    loadEntries();

    return {
      zoneKind,
      load: (prefix) => loadEntries(prefix || ''),
      refresh: () => loadEntries(state.prefix),
      getSelection: () => Array.from(state.selection),
      clearSelection,
      getPrefix: () => state.prefix,
      setDragDropEnabled: (enabled) => {
        state.dragDropEnabled = !!enabled;
        root.classList.toggle('zone-browser-dnd-disabled', !state.dragDropEnabled);
        renderEntries();
      }
    };
  }

  async function refreshZoneBrowsers() {
    const browsers = Object.values(zoneBrowserInstances).filter(Boolean);
    await Promise.all(browsers.map((browser) => (browser && browser.refresh ? browser.refresh() : Promise.resolve())));
  }

  function shouldAllowDragDrop() {
    return getFlowSource() === 'user';
  }

  function shouldAutoApproveDropActions() {
    return shouldAllowDragDrop() && getDragDropAutoApprovePreference();
  }

  function autoApprovePayload(autoApprove) {
    if (!autoApprove) {
      return { auto_approve: false };
    }
    return {
      auto_approve: true,
      auto_approve_origin: 'control_panel_user'
    };
  }

  function buildDestinationPath(destPrefix, srcPath) {
    const normalizedDest = normalizePrefix(destPrefix || '');
    if (!normalizedDest) return srcPath;
    return `${normalizedDest}/${srcPath}`;
  }

  function buildOptionalDeliveryDestination(destPrefix, stageRef) {
    const normalizedDest = normalizePrefix(destPrefix || '');
    if (!normalizedDest) return null;
    return buildDestinationPath(normalizedDest, stageRef);
  }

  async function stagePaths(paths, destPrefix, options = {}) {
    if (!paths.length) {
      toast('Select at least one workspace item to stage', 'warning');
      return;
    }
    const autoApprove = options.dropTriggered ? shouldAutoApproveDropActions() : getFlowSource() === 'user';
    const runner = normalizeImportExportRunner(importExportRunnerSelection);
    const normalizedPaths = paths.map((path) => normalizePrefix(path)).filter(Boolean);
    if (!normalizedPaths.length) {
      toast('No valid workspace entries selected', 'warning');
      return;
    }
    const summary = { success: 0, pending: 0, errors: [] };
    for (const src of normalizedPaths) {
      try {
        const response = await api('/api/export/request', {
          method: 'POST',
          body: {
            src,
            dst: buildDestinationPath(destPrefix, src),
            runner,
            ...autoApprovePayload(autoApprove)
          }
        });
        if (response.status === 'pending_approval') summary.pending += 1;
        else if (response.status === 'staged') summary.success += 1;
      } catch (err) {
        summary.errors.push({ src, err });
      }
    }
    if (summary.success) {
      toast(`Staged ${summary.success} item(s)`, 'success');
    }
    if (summary.pending) {
      toast(`Staging queued for approval (${summary.pending})`, 'info', 6000, {
        linkHref: '/approvals',
        linkLabel: 'Review approvals'
      });
    }
    if (summary.errors.length) {
      const first = summary.errors[0];
      toast(`Failed to stage ${first.src}: ${first.err.message || first.err}`, 'error');
    }
    await Promise.all([refreshStatus(), refreshZoneBrowsers()]);
  }

  async function deliverPaths(paths, destPrefix, options = {}) {
    if (!paths.length) {
      toast('Select at least one shared zone item to deliver', 'warning');
      return;
    }
    const autoApprove = options.dropTriggered ? shouldAutoApproveDropActions() : getFlowSource() === 'user';
    const runner = normalizeImportExportRunner(importExportRunnerSelection);
    const summary = { success: 0, pending: 0, errors: [] };
    for (const stageRef of paths) {
      try {
        const response = await api('/api/export/deliver/request', {
          method: 'POST',
          body: {
            stage_ref: stageRef,
            dst: buildOptionalDeliveryDestination(destPrefix, stageRef),
            move_artifact: false,
            runner,
            ...autoApprovePayload(autoApprove)
          }
        });
        if (response.status === 'pending_approval') summary.pending += 1;
        else if (response.status === 'delivered') summary.success += 1;
      } catch (err) {
        summary.errors.push({ stageRef, err });
      }
    }
    if (summary.success) {
      toast(`Delivered ${summary.success} item(s)`, 'success');
    }
    if (summary.pending) {
      toast(`Delivery queued for approval (${summary.pending})`, 'info', 6000, {
        linkHref: '/approvals',
        linkLabel: 'Review approvals'
      });
    }
    if (summary.errors.length) {
      const first = summary.errors[0];
      toast(`Failed to deliver ${first.stageRef}: ${first.err.message || first.err}`, 'error');
    }
    await Promise.all([refreshStatus(), refreshZoneBrowsers()]);
  }

  function renderFiles(root) {
    const source = getFlowSource();
    const preview = getPreviewMode();
    const dragDropAutoApprove = getDragDropAutoApprovePreference();
    let selectedRunner = normalizeImportExportRunner(importExportRunnerSelection) || preferredImportExportRunner();
    importExportRunnerSelection = selectedRunner;
    const runnerOptions = runnerFilterOptions().filter((option) => option.id !== 'all');

    root.innerHTML = `
      <div class="settings-container" style="max-width: 1400px; padding: 0 var(--space-4);">
        <div class="settings-header" style="margin-bottom: var(--space-6); padding-bottom: var(--space-6); border-bottom: 1px solid var(--content-border);">
          <h2 class="settings-title">Import / Export Data</h2>
          <p class="settings-description">Transfer files across isolation boundaries and manage external deliveries.</p>
        </div>

        <div style="background: var(--content-bg); border: 1px solid var(--content-border); border-radius: var(--radius-lg); padding: var(--space-4); margin-bottom: var(--space-6);">
          <div style="display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: var(--space-4);">
            <div>
              <h3 style="font-size: 1.05rem; font-weight: 600; color: var(--text-primary); margin: 0 0 var(--space-1) 0;">Transfer Flow</h3>
              <p style="font-size: 0.9rem; color: var(--text-muted); margin: 0;">Configure how files move between zones.</p>
            </div>
            <div class="btn-group" style="display: flex; gap: var(--space-2);">
              <button id="btn-import" class="btn btn-primary btn-sm">Import</button>
              <button id="btn-stage" class="btn btn-secondary btn-sm">Stage</button>
              <button id="btn-deliver" class="btn btn-secondary btn-sm">Deliver</button>
            </div>
          </div>
          
          <div style="display: flex; gap: var(--space-6); flex-wrap: wrap;">
            <div class="form-group" style="flex: 1; min-width: 200px; margin: 0;">
              <label class="form-label">Runner Context</label>
              <select id="files-runner-context" class="form-select">
                ${runnerOptions.map((option) => `
                  <option value="${esc(option.id)}" ${selectedRunner === option.id ? 'selected' : ''}>${esc(option.label)}</option>
                `).join('')}
              </select>
            </div>
            <div class="form-group" style="flex: 1; min-width: 200px; margin: 0;">
              <label class="form-label">Action Source</label>
              <select id="flow-source" class="form-select">
                <option value="user" ${source === 'user' ? 'selected' : ''}>User (auto-approve)</option>
                <option value="agent" ${source === 'agent' ? 'selected' : ''}>Agent (approval queue)</option>
              </select>
            </div>
            <div class="form-group" style="flex: 1; min-width: 200px; margin: 0;">
              <label class="form-label">Preview Mode</label>
              <select id="preview-mode" class="form-select">
                <option value="always" ${preview === 'always' ? 'selected' : ''}>Always preview</option>
                <option value="auto" ${preview === 'auto' ? 'selected' : ''}>Auto (preview for agent)</option>
                <option value="off" ${preview === 'off' ? 'selected' : ''}>Skip preview</option>
              </select>
            </div>
          </div>
          
          <div id="drag-drop-auto-approve-row" class="mt-4 ${source === 'user' ? '' : 'hidden'}" style="padding-top: var(--space-4); border-top: 1px dashed var(--content-border);">
            <label class="form-switch">
              <input type="checkbox" id="drag-drop-auto-approve" class="form-switch-input" ${dragDropAutoApprove ? 'checked' : ''} />
              <div class="form-switch-text">
                <span class="form-switch-label">Auto-approve drag/drop transfers (user-only)</span>
                <span class="form-switch-description">Drag/drop is unavailable when Action Source is set to Agent.</span>
              </div>
            </label>
          </div>
        </div>

        <div style="background: var(--info-bg); border: 1px solid var(--info-border); border-radius: var(--radius-lg); padding: var(--space-3) var(--space-4); margin-bottom: var(--space-6); display: flex; gap: var(--space-3); align-items: flex-start;">
          <div style="color: var(--info); font-size: 1.2rem;">ℹ</div>
          <div>
            <div style="font-weight: 600; font-size: 0.9rem; color: var(--info);">Zone explorer tip</div>
            <div style="font-size: 0.85rem; color: var(--text-primary); margin-top: 2px;">Scroll inside each zone browser to inspect nested directories. Drag and drop to quickly stage or deliver.</div>
          </div>
        </div>

        <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: var(--space-6); padding-bottom: var(--space-8);">
          
          <div style="display: flex; flex-direction: column; border: 1px solid var(--content-border); border-radius: var(--radius-lg); overflow: hidden; background: var(--content-bg);">
            <div style="padding: var(--space-4); border-bottom: 1px solid var(--content-border); background: var(--content-bg-alt);">
              <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: var(--space-2);">
                <h3 style="font-size: 1.05rem; font-weight: 600; margin: 0; color: var(--text-primary);">Workspace</h3>
                <span class="chip chip-success" style="margin: 0;">Zone 0</span>
              </div>
              <p style="font-size: 0.85rem; color: var(--text-muted); margin: 0 0 var(--space-1) 0;">Agent working directory.</p>
              <p id="zone-path-workspace" class="form-hint mono" style="margin: 0; word-break: break-all;">-</p>
            </div>
            <div style="display: flex; flex: 1;">
              <div id="zone-browser-workspace" style="width: 100%;"></div>
            </div>
          </div>
          
          <div style="display: flex; flex-direction: column; border: 1px solid var(--warning-border); border-radius: var(--radius-lg); overflow: hidden; background: var(--content-bg);">
            <div style="padding: var(--space-4); border-bottom: 1px solid var(--warning-border); background: color-mix(in srgb, var(--warning-bg) 50%, var(--content-bg-alt));">
              <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: var(--space-2);">
                <h3 style="font-size: 1.05rem; font-weight: 600; margin: 0; color: var(--text-primary);">Shared Zone</h3>
                <span class="chip chip-warning" style="margin: 0;">Zone 2</span>
              </div>
              <p style="font-size: 0.85rem; color: var(--text-muted); margin: 0 0 var(--space-1) 0;">Staged exports awaiting delivery.</p>
              <p id="zone-path-shared" class="form-hint mono" style="margin: 0; word-break: break-all;">-</p>
            </div>
            <div style="display: flex; flex: 1;">
              <div id="zone-browser-shared" style="width: 100%;"></div>
            </div>
          </div>
          
          <div style="display: flex; flex-direction: column; border: 1px solid var(--brand-border); border-radius: var(--radius-lg); overflow: hidden; background: var(--content-bg);">
            <div style="padding: var(--space-4); border-bottom: 1px solid var(--brand-border); background: color-mix(in srgb, var(--brand-primary) 10%, var(--content-bg-alt));">
              <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: var(--space-2);">
                <h3 style="font-size: 1.05rem; font-weight: 600; margin: 0; color: var(--text-primary);">Deliveries</h3>
                <span class="chip chip-primary" style="margin: 0;">User Dest</span>
              </div>
              <p style="font-size: 0.85rem; color: var(--text-muted); margin: 0 0 var(--space-1) 0;">Final delivery destination.</p>
              <p id="zone-path-deliver" class="form-hint mono" style="margin: 0; word-break: break-all;">-</p>
            </div>
            <div style="display: flex; flex: 1;">
              <div id="zone-browser-deliver" style="width: 100%;"></div>
            </div>
          </div>
          
        </div>
      </div>
    `;

    const runnerContextSelect = document.getElementById('files-runner-context');
    const workspacePathEl = document.getElementById('zone-path-workspace');
    const sharedPathEl = document.getElementById('zone-path-shared');
    const deliverPathEl = document.getElementById('zone-path-deliver');

    const applyRunnerPathHints = (runtimePayload) => {
      if (workspacePathEl) {
        workspacePathEl.textContent = aliasRuntimePath(runtimePayload?.workspace || '-');
      }
      if (sharedPathEl) {
        sharedPathEl.textContent = aliasRuntimePath(runtimePayload?.shared_zone || '-');
      }
      if (deliverPathEl) {
        deliverPathEl.textContent = aliasRuntimePath(runtimePayload?.default_user_destination_dir || runtimePayload?.default_delivery_dir || '-');
      }
    };

    const refreshRunnerPathHints = async () => {
      const runnerId = normalizeImportExportRunner(importExportRunnerSelection) || preferredImportExportRunner();
      const runtimePath = `/api/runtime?runner=${encodeURIComponent(runnerId)}`;
      try {
        const runtimePayload = await api(runtimePath);
        applyRunnerPathHints(runtimePayload);
      } catch (err) {
        applyRunnerPathHints(null);
        toast(`Failed to refresh runner paths: ${err.message}`, 'warning');
      }
    };

    const workspaceContainer = document.getElementById('zone-browser-workspace');
    const sharedContainer = document.getElementById('zone-browser-shared');
    const deliverContainer = document.getElementById('zone-browser-deliver');

    zoneBrowserInstances.workspace = createZoneFileBrowser({
      container: workspaceContainer,
      zoneKind: 'workspace',
      title: 'Workspace Explorer',
      chipLabel: 'Zone 0',
      selectionMode: 'multi',
      runnerProvider: () => importExportRunnerSelection
    });

    zoneBrowserInstances.shared = createZoneFileBrowser({
      container: sharedContainer,
      zoneKind: 'shared',
      title: 'Shared Zone Explorer',
      chipLabel: 'Zone 2',
      selectionMode: 'multi',
      runnerProvider: () => importExportRunnerSelection,
      dropConfig: {
        from: 'workspace',
        hint: 'Drop workspace items here to stage them.',
        invalidMessage: 'Only workspace entries can be dropped here.',
        handler: (paths, prefix) => stagePaths(paths, prefix, { dropTriggered: true })
      }
    });

    zoneBrowserInstances.deliver = createZoneFileBrowser({
      container: deliverContainer,
      zoneKind: 'deliver',
      title: 'Deliveries Explorer',
      chipLabel: 'User Destination',
      selectionMode: 'multi',
      runnerProvider: () => importExportRunnerSelection,
      dropConfig: {
        from: 'shared',
        hint: 'Drop shared zone items here to deliver them.',
        invalidMessage: 'Only shared zone entries can be dropped here.',
        handler: (paths, prefix) => deliverPaths(paths, prefix, { dropTriggered: true })
      }
    });

    if (runnerContextSelect) {
      runnerContextSelect.addEventListener('change', async (event) => {
        const nextRunner = normalizeImportExportRunner(event.target.value) || preferredImportExportRunner();
        selectedRunner = nextRunner;
        importExportRunnerSelection = nextRunner;
        localStorage.setItem(IMPORT_EXPORT_RUNNER_STORAGE_KEY, nextRunner);
        await refreshRunnerPathHints();
        await Promise.all([
          zoneBrowserInstances.workspace?.load('') || Promise.resolve(),
          zoneBrowserInstances.shared?.load('') || Promise.resolve(),
          zoneBrowserInstances.deliver?.load('') || Promise.resolve()
        ]);
      });
    }
    refreshRunnerPathHints();

    const flowSourceSelect = document.getElementById('flow-source');
    const dragDropRow = document.getElementById('drag-drop-auto-approve-row');
    const dragDropCheckbox = document.getElementById('drag-drop-auto-approve');
    const setDragDropVisibility = (isUser) => {
      if (dragDropRow) {
        dragDropRow.classList.toggle('hidden', !isUser);
      }
      if (dragDropCheckbox) {
        dragDropCheckbox.disabled = !isUser;
      }
      Object.values(zoneBrowserInstances).forEach((browser) => {
        if (browser && typeof browser.setDragDropEnabled === 'function') {
          browser.setDragDropEnabled(isUser);
        }
      });
    };

    if (dragDropCheckbox) {
      dragDropCheckbox.addEventListener('change', (event) => {
        setDragDropAutoApprovePreference(!!event.target.checked);
      });
    }

    if (flowSourceSelect) {
      flowSourceSelect.addEventListener('change', (event) => {
        const value = event.target.value;
        setFlowSource(value);
        toast('Action source updated', 'info');
        setDragDropVisibility(value === 'user');
      });
    }
    setDragDropVisibility(source === 'user');

    document.getElementById('preview-mode').addEventListener('change', (event) => {
      setPreviewMode(event.target.value);
      toast('Preview mode updated', 'info');
    });

    document.getElementById('btn-import').addEventListener('click', openImportModal);

    document.getElementById('btn-stage').addEventListener('click', () => {
      const selection = zoneBrowserInstances.workspace?.getSelection() || [];
      if (selection.length) {
        stagePaths(selection, zoneBrowserInstances.shared?.getPrefix());
        return;
      }
      openStageModal();
    });

    document.getElementById('btn-deliver').addEventListener('click', () => {
      const selection = zoneBrowserInstances.shared?.getSelection() || [];
      if (selection.length) {
        deliverPaths(selection, zoneBrowserInstances.deliver?.getPrefix());
        return;
      }
      openDeliverModal();
    });
  }

  function openImportModal() {
    openModal('Import Into Workspace', `
      <p class="text-muted mb-4">Import files from external sources into the workspace zone.</p>
      
      <div class="alert alert-info mb-4">
        <span class="alert-icon">ℹ</span>
        <div class="alert-content">
          <div class="alert-message">Flow: External Source → Workspace</div>
        </div>
      </div>
      
      <div class="form-group">
        <label class="form-label">Source File</label>
        <input type="file" id="import-file" class="form-input" />
      </div>
      
      <div class="form-group">
        <label class="form-check">
          <input type="checkbox" id="import-custom-toggle" class="form-check-input" />
          <span class="form-check-label">Custom destination path</span>
        </label>
      </div>
      
      <div id="import-custom-options" class="hidden">
        <div class="form-group">
          <label class="form-label">Destination Path</label>
          <input type="text" id="import-dst" class="form-input" placeholder="Relative to workspace" />
        </div>
      </div>
      
      <div id="import-preview" class="diff-preview hidden mt-4"></div>
    `, {
      footer: `
        <button class="btn btn-ghost" onclick="closeModal()">Cancel</button>
        <button id="import-submit" class="btn btn-primary">Import</button>
      `
    });
    
    document.getElementById('import-custom-toggle').addEventListener('change', (e) => {
      document.getElementById('import-custom-options').classList.toggle('hidden', !e.target.checked);
    });
    
    document.getElementById('import-submit').addEventListener('click', submitImport);
  }

  async function submitImport() {
    const fileInput = document.getElementById('import-file');
    const customToggle = document.getElementById('import-custom-toggle');
    const customDstInput = document.getElementById('import-dst');
    const useCustomDst = !!customToggle?.checked;
    const customDst = (customDstInput?.value || '').trim();

    if (!fileInput.files.length) {
      toast('Please select a file', 'warning');
      return;
    }
    if (useCustomDst && !customDst) {
      toast('Please provide a destination path or disable custom destination', 'warning');
      return;
    }

    const submitBtn = document.getElementById('import-submit');
    if (submitBtn) submitBtn.disabled = true;

    try {
      const uploaded = await uploadImportFile(fileInput.files[0]);
      const autoApprove = getFlowSource() === 'user';
      const dst = useCustomDst ? customDst : (uploaded.suggested_dst || null);
      const runner = normalizeImportExportRunner(importExportRunnerSelection);
      const result = await api('/api/import/request', {
        method: 'POST',
        body: {
          src: uploaded.uploaded_src,
          dst,
          runner,
          ...autoApprovePayload(autoApprove)
        }
      });

      if (result.status === 'completed') {
        toast('File imported successfully', 'success');
      } else if (result.status === 'pending_approval') {
        toast('Import queued for approval', 'info', 6000, {
          linkHref: '/approvals',
          linkLabel: 'Review approvals'
        });
      }
      closeModal();
      await Promise.all([refreshStatus(), refreshZoneBrowsers()]);
    } catch (err) {
      toast(`Import failed: ${err.message}`, 'error');
    } finally {
      if (submitBtn) submitBtn.disabled = false;
    }
  }

  function openStageModal() {
    openModal('Stage to Shared Zone', `
      <p class="text-muted mb-4">Stage workspace files to the shared zone for delivery.</p>
      
      <div class="alert alert-warning mb-4">
        <span class="alert-icon">⚠</span>
        <div class="alert-content">
          <div class="alert-message">Flow: Workspace → Shared Zone (approval may be required)</div>
        </div>
      </div>
      
      <div class="form-group">
        <label class="form-label">Source Path (in workspace)</label>
        <input type="text" id="stage-src" class="form-input" placeholder="e.g., output/result.txt" />
      </div>
      
      <div class="form-group">
        <label class="form-check">
          <input type="checkbox" id="stage-custom-toggle" class="form-check-input" />
          <span class="form-check-label">Custom destination in shared zone</span>
        </label>
      </div>

      <div id="stage-custom-options" class="hidden">
        <div class="form-group">
          <label class="form-label">Destination</label>
          <input type="text" id="stage-dst" class="form-input" placeholder="Defaults to same relative path in shared zone" />
        </div>
      </div>

      <p class="form-hint">Use the workspace explorer above to pick files you want to stage, or enter the exact path.</p>
      
      <div id="stage-preview" class="diff-preview hidden mt-4"></div>
    `, {
      footer: `
        <button class="btn btn-ghost" onclick="closeModal()">Cancel</button>
        <button id="stage-preview-btn" class="btn btn-secondary">Preview</button>
        <button id="stage-submit" class="btn btn-primary">Stage</button>
      `
    });
    
    document.getElementById('stage-custom-toggle').addEventListener('change', (e) => {
      document.getElementById('stage-custom-options').classList.toggle('hidden', !e.target.checked);
    });

    document.getElementById('stage-preview-btn').addEventListener('click', previewStage);
    document.getElementById('stage-submit').addEventListener('click', submitStage);
  }

  async function previewStage() {
    const src = (document.getElementById('stage-src').value || '').trim();
    const useCustomDst = document.getElementById('stage-custom-toggle')?.checked;
    const customDst = (document.getElementById('stage-dst')?.value || '').trim();
    if (!src) {
      toast('Please enter a source path', 'warning');
      return;
    }
    if (useCustomDst && !customDst) {
      toast('Please provide a destination path or disable custom destination', 'warning');
      return;
    }
    
    try {
      const runner = normalizeImportExportRunner(importExportRunnerSelection);
      const result = await api('/api/export/preview', {
        method: 'POST',
        body: { src, dst: useCustomDst ? customDst : null, runner }
      });
      
      const preview = document.getElementById('stage-preview');
      preview.classList.remove('hidden');
      preview.innerHTML = `
        <div class="diff-summary">${formatDiff(result.summary)}</div>
        <div class="mt-2">
          <strong>From:</strong> <span class="mono">${esc(result.src)}</span><br>
          <strong>To:</strong> <span class="mono">${esc(result.dst)}</span>
        </div>
      `;
    } catch (err) {
      toast(`Preview failed: ${err.message}`, 'error');
    }
  }

  async function submitStage() {
    const src = (document.getElementById('stage-src').value || '').trim();
    const useCustomDst = document.getElementById('stage-custom-toggle')?.checked;
    const customDst = (document.getElementById('stage-dst')?.value || '').trim();
    const dst = useCustomDst ? customDst : null;
    
    if (!src) {
      toast('Please enter a source path', 'warning');
      return;
    }
    if (useCustomDst && !dst) {
      toast('Please provide a destination path or disable custom destination', 'warning');
      return;
    }
    
    try {
      const autoApprove = getFlowSource() === 'user';
      const runner = normalizeImportExportRunner(importExportRunnerSelection);
      const result = await api('/api/export/request', {
        method: 'POST',
        body: { src, dst, runner, ...autoApprovePayload(autoApprove) }
      });
      
      if (result.status === 'staged') {
        toast('File staged successfully', 'success');
        closeModal();
        await refreshZoneBrowsers();
      } else if (result.status === 'pending_approval') {
        toast('Staging queued for approval', 'info', 6000, {
          linkHref: '/approvals',
          linkLabel: 'Review approvals'
        });
        closeModal();
      }
    } catch (err) {
      toast(`Stage failed: ${err.message}`, 'error');
    }
  }

  function openDeliverModal() {
    openModal('Deliver to User Destination', `
      <p class="text-muted mb-4">Deliver staged files to the final user destination.</p>
      
      <div class="alert alert-warning mb-4">
        <span class="alert-icon">⚠</span>
        <div class="alert-content">
          <div class="alert-message">Flow: Shared Zone → User Destination (approval required)</div>
        </div>
      </div>
      
      <div class="form-group">
        <label class="form-label">Stage Reference</label>
        <input type="text" id="deliver-ref" class="form-input" placeholder="File path in shared zone or stage ID" />
      </div>

      <p class="form-hint">Pick stage references using the shared zone explorer, then confirm delivery below.</p>
      
      <div class="form-group">
        <label class="form-check">
          <input type="checkbox" id="deliver-custom-toggle" class="form-check-input" />
          <span class="form-check-label">Custom destination path</span>
        </label>
      </div>

      <div id="deliver-custom-options" class="hidden">
        <div class="form-group">
          <label class="form-label">Destination</label>
          <input type="text" id="deliver-dst" class="form-input" placeholder="Absolute path or relative name" />
        </div>
      </div>
      
      <div class="form-group">
        <label class="form-check">
          <input type="checkbox" id="deliver-move" class="form-check-input" />
          <span class="form-check-label">Move (remove from shared zone after delivery)</span>
        </label>
      </div>
      
      <div id="deliver-preview" class="diff-preview hidden mt-4"></div>
    `, {
      footer: `
        <button class="btn btn-ghost" onclick="closeModal()">Cancel</button>
        <button id="deliver-preview-btn" class="btn btn-secondary">Preview</button>
        <button id="deliver-submit" class="btn btn-warning">Deliver</button>
      `
    });
    
    document.getElementById('deliver-custom-toggle').addEventListener('change', (e) => {
      document.getElementById('deliver-custom-options').classList.toggle('hidden', !e.target.checked);
    });
    document.getElementById('deliver-preview-btn').addEventListener('click', previewDeliver);
    document.getElementById('deliver-submit').addEventListener('click', submitDeliver);
  }

  async function previewDeliver() {
    const ref = (document.getElementById('deliver-ref').value || '').trim();
    const useCustomDst = document.getElementById('deliver-custom-toggle')?.checked;
    const customDst = (document.getElementById('deliver-dst')?.value || '').trim();

    if (!ref) {
      toast('Please enter a stage reference', 'warning');
      return;
    }
    if (useCustomDst && !customDst) {
      toast('Please provide a destination path or disable custom destination', 'warning');
      return;
    }
    
    try {
      const runner = normalizeImportExportRunner(importExportRunnerSelection);
      const result = await api('/api/export/deliver/preview', {
        method: 'POST',
        body: { stage_ref: ref, dst: useCustomDst ? customDst : null, runner }
      });
      
      const preview = document.getElementById('deliver-preview');
      preview.classList.remove('hidden');
      preview.innerHTML = `
        <div class="diff-summary">${formatDiff(result.summary)}</div>
        <div class="mt-2">
          <strong>From:</strong> <span class="mono">${esc(result.src)}</span><br>
          <strong>To:</strong> <span class="mono">${esc(result.dst)}</span>
        </div>
      `;
    } catch (err) {
      toast(`Preview failed: ${err.message}`, 'error');
    }
  }

  async function submitDeliver() {
    const ref = (document.getElementById('deliver-ref').value || '').trim();
    const useCustomDst = document.getElementById('deliver-custom-toggle').checked;
    const customDst = (document.getElementById('deliver-dst').value || '').trim();
    const dst = useCustomDst ? (customDst || null) : null;
    const move = document.getElementById('deliver-move').checked;
    
    if (!ref) {
      toast('Please enter a stage reference', 'warning');
      return;
    }
    if (useCustomDst && !dst) {
      toast('Please provide a destination path or disable custom destination', 'warning');
      return;
    }
    
    try {
      const autoApprove = getFlowSource() === 'user';
      const runner = normalizeImportExportRunner(importExportRunnerSelection);
      const result = await api('/api/export/deliver/request', {
        method: 'POST',
        body: {
          stage_ref: ref,
          dst,
          move_artifact: move,
          runner,
          ...autoApprovePayload(autoApprove)
        }
      });
      
      if (result.status === 'delivered') {
        toast('File delivered successfully', 'success');
        closeModal();
        await Promise.all([refreshStatus(), refreshZoneBrowsers()]);
      } else if (result.status === 'pending_approval') {
        toast('Delivery queued for approval', 'info', 6000, {
          linkHref: '/approvals',
          linkLabel: 'Review approvals'
        });
        closeModal();
      }
    } catch (err) {
      toast(`Deliver failed: ${err.message}`, 'error');
    }
  }
