  const RUNNERS_VIEW_STORAGE_KEY = 'ar.runners.view';
  const RUNNERS_VIEW_OPTIONS = [
    { id: 'all', label: 'All' },
    { id: 'openclaw', label: 'OpenClaw' },
    { id: 'claudecode', label: 'Claude Code' },
    { id: 'opencode', label: 'OpenCode' }
  ];
  const RUNNER_SESSIONS_DEFAULT_LIMIT = 25;
  const RUNNER_SESSION_STATUS_OPTIONS = [
    { id: 'active', label: 'Active' },
    { id: 'archived', label: 'Archived' },
    { id: 'all', label: 'All statuses' }
  ];
  const RUNNER_SESSION_ACTIVITY_OPTIONS = [
    { id: 'recent', label: 'Recent (7d)' },
    { id: 'all', label: 'All activity' }
  ];
  const RUNNER_SESSION_CHANNEL_OPTIONS = [
    { id: 'all', label: 'All channels' },
    { id: 'telegram', label: 'Telegram' },
    { id: 'tui', label: 'TUI' },
    { id: 'web', label: 'Web' },
    { id: 'api', label: 'API' }
  ];
  let runnersFleetCache = null;
  let runnersFleetRequest = null;
  let runnersLoadRequestId = 0;
  let runnerSessionsLoadRequestId = 0;
  let runnerSessionsSearchDebounce = null;

  const runnerSessionsState = {
    items: [],
    total: 0,
    hasMore: false,
    nextCursor: null,
    filters: {
      q: '',
      status: 'active',
      activity: 'recent',
      channel: 'all'
    }
  };

  function normalizeRunnersView(value) {
    const normalized = String(value || '').trim().toLowerCase();
    if (RUNNERS_VIEW_OPTIONS.some((option) => option.id === normalized)) return normalized;
    return 'all';
  }

  function renderRunners(root) {
    root.innerHTML = `
      <div class="settings-container" style="max-width: 1400px; padding: 0 var(--space-4);">
        <div class="settings-header" style="margin-bottom: var(--space-6); padding-bottom: var(--space-6); border-bottom: 1px solid var(--content-border);">
          <div>
            <h2 class="settings-title">Runner Fleet & Sessions</h2>
            <p class="settings-description">Monitor active agents and review session history</p>
          </div>
          <div style="display: flex; gap: var(--space-2);">
            <button id="runners-refresh" class="btn btn-sm btn-outline" type="button"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="margin-right: 4px;"><path d="M21 2v6h-6"/><path d="M3 12a9 9 0 1 0 2.6-6.4L2 9"/></svg> Refresh</button>
          </div>
        </div>

        <div class="panel-tabs" id="main-runners-tabs" role="tablist" style="margin-bottom: var(--space-6);">
          <button type="button" class="panel-tab active" data-main-tab="fleet">Fleet</button>
          <button type="button" class="panel-tab" data-main-tab="sessions">Sessions</button>
        </div>

        <div id="tab-content-fleet" class="settings-section" style="display: block;">
          <div class="settings-row" style="margin-bottom: var(--space-6);">
            <div class="settings-row-info">
              <h3>Runner Fleet</h3>
              <p>Installed status, health handshakes, capabilities, and managed config visibility per runner.</p>
            </div>
            <div class="settings-row-control" style="justify-content: flex-end;">
              <div class="panel-tabs" id="runners-tab-list" role="tablist" aria-label="Runner fleet filters">
                ${RUNNERS_VIEW_OPTIONS.map((option) => `
                  <button type="button" class="panel-tab" data-runners-tab="${esc(option.id)}" role="tab">${esc(option.label)}</button>
                `).join('')}
              </div>
            </div>
          </div>
          <p id="runners-filter-summary" class="form-hint mb-4"></p>
          <div id="runners-grid"></div>
        </div>

        <div id="tab-content-sessions" class="settings-section" style="display: none;">
          <div class="settings-row" style="margin-bottom: var(--space-4);">
            <div class="settings-row-info">
              <h3>Recent Sessions</h3>
              <p>Find runner-bound sessions without loading the full history up front. Search first, then load more only when needed.</p>
            </div>
          </div>
          
          <div class="grid grid-4" style="margin-bottom: var(--space-4);">
            <div class="form-group">
              <label class="form-label" for="runner-sessions-search">Search</label>
              <input id="runner-sessions-search" class="form-input" type="search" placeholder="Session id, label, thread, key" value="${esc(runnerSessionsState.filters.q)}" />
            </div>
            <div class="form-group">
              <label class="form-label" for="runner-sessions-status">Status</label>
              <select id="runner-sessions-status" class="form-select">
                ${RUNNER_SESSION_STATUS_OPTIONS.map((option) => `
                  <option value="${esc(option.id)}" ${runnerSessionsState.filters.status === option.id ? 'selected' : ''}>${esc(option.label)}</option>
                `).join('')}
              </select>
            </div>
            <div class="form-group">
              <label class="form-label" for="runner-sessions-activity">Activity</label>
              <select id="runner-sessions-activity" class="form-select">
                ${RUNNER_SESSION_ACTIVITY_OPTIONS.map((option) => `
                  <option value="${esc(option.id)}" ${runnerSessionsState.filters.activity === option.id ? 'selected' : ''}>${esc(option.label)}</option>
                `).join('')}
              </select>
            </div>
            <div class="form-group">
              <label class="form-label" for="runner-sessions-channel">Channel</label>
              <select id="runner-sessions-channel" class="form-select">
                ${RUNNER_SESSION_CHANNEL_OPTIONS.map((option) => `
                  <option value="${esc(option.id)}" ${runnerSessionsState.filters.channel === option.id ? 'selected' : ''}>${esc(option.label)}</option>
                `).join('')}
              </select>
            </div>
          </div>
          
          <p id="runner-sessions-summary" class="form-hint mb-4"></p>
          <div style="background: var(--content-bg-alt); border: 1px solid var(--content-border); border-radius: var(--radius-lg); box-shadow: var(--shadow-sm); overflow: hidden;">
            <div id="runner-sessions-list" style="max-height: 600px; overflow-y: auto;"></div>
          </div>
          
          <div class="mt-4" style="display: flex; justify-content: center;">
            <button id="runner-sessions-load-more" class="btn btn-ghost" type="button">Load more</button>
          </div>
        </div>
      </div>
    `;

    // Main Tabs Logic
    const mainTabs = document.querySelectorAll('[data-main-tab]');
    const fleetContent = document.getElementById('tab-content-fleet');
    const sessionsContent = document.getElementById('tab-content-sessions');
    
    mainTabs.forEach(tab => {
      tab.addEventListener('click', () => {
        mainTabs.forEach(t => t.classList.remove('active'));
        tab.classList.add('active');
        const target = tab.getAttribute('data-main-tab');
        if (target === 'fleet') {
          fleetContent.style.display = 'block';
          sessionsContent.style.display = 'none';
        } else {
          fleetContent.style.display = 'none';
          sessionsContent.style.display = 'block';
        }
      });
    });

    document.getElementById('runners-refresh').addEventListener('click', () => {
      loadRunners({ force: true });
      loadRunnerSessions({ reset: true });
    });

    Array.from(document.querySelectorAll('[data-runners-tab]')).forEach((button) => {
      button.addEventListener('click', () => {
        const nextView = normalizeRunnersView(button.getAttribute('data-runners-tab'));
        localStorage.setItem(RUNNERS_VIEW_STORAGE_KEY, nextView);
        updateRunnerTabState(nextView);
        if (Array.isArray(runnersFleetCache)) {
          renderRunnerCards(runnersFleetCache);
        } else {
          loadRunners();
        }
        loadRunnerSessions({ reset: true });
      });
    });

    document.getElementById('runner-sessions-search').addEventListener('input', (event) => {
      runnerSessionsState.filters.q = String(event.target.value || '');
      if (runnerSessionsSearchDebounce) clearTimeout(runnerSessionsSearchDebounce);
      runnerSessionsSearchDebounce = setTimeout(() => {
        loadRunnerSessions({ reset: true });
      }, 220);
    });
    document.getElementById('runner-sessions-status').addEventListener('change', (event) => {
      runnerSessionsState.filters.status = String(event.target.value || 'active');
      loadRunnerSessions({ reset: true });
    });
    document.getElementById('runner-sessions-activity').addEventListener('change', (event) => {
      runnerSessionsState.filters.activity = String(event.target.value || 'recent');
      loadRunnerSessions({ reset: true });
    });
    document.getElementById('runner-sessions-channel').addEventListener('change', (event) => {
      runnerSessionsState.filters.channel = String(event.target.value || 'all');
      loadRunnerSessions({ reset: true });
    });
    document.getElementById('runner-sessions-load-more').addEventListener('click', () => {
      if (!runnerSessionsState.hasMore) return;
      loadRunnerSessions({ reset: false });
    });

    const initialView = normalizeRunnersView(localStorage.getItem(RUNNERS_VIEW_STORAGE_KEY));
    updateRunnerTabState(initialView);
    loadRunners();
    loadRunnerSessions({ reset: true });
  }

  function updateRunnerTabState(activeView) {
    Array.from(document.querySelectorAll('[data-runners-tab]')).forEach((button) => {
      const isActive = button.getAttribute('data-runners-tab') === activeView;
      button.classList.toggle('active', isActive);
      button.setAttribute('aria-selected', isActive ? 'true' : 'false');
    });
  }

  function activeRunnerSessionsFilter() {
    const activeView = normalizeRunnersView(localStorage.getItem(RUNNERS_VIEW_STORAGE_KEY));
    return activeView === 'all' ? '' : activeView;
  }

  function runnerSessionRow(item) {
    const channels = Array.isArray(item?.channels) ? item.channels : [];
    const runnerLabel = item?.runner_label || item?.runner_kind || 'runner';
    const status = String(item?.status || 'active').toLowerCase();
    const statusClass = status === 'archived' ? 'chip-warning' : 'chip-success';
    const threadChip = Number.isInteger(item?.telegram_thread_id)
      ? `<span class="chip">thread ${esc(item.telegram_thread_id)}</span>`
      : '';
    const label = item?.display_label || item?.label || item?.title || item?.id || 'session';
    return `
      <div class="list-item">
        <div class="list-item-content">
          <div class="list-item-title">${esc(label)}</div>
          <div class="list-item-description">
            <span class="chip">${esc(runnerLabel)}</span>
            <span class="chip ${statusClass}">${esc(status)}</span>
            ${channels.map((channel) => `<span class="chip">${esc(channel)}</span>`).join(' ')}
            ${threadChip}
          </div>
          <div class="form-hint">Last active ${esc(formatRelativeTime(item?.last_active_at))}</div>
          <div class="form-hint mono">${esc(item?.id || '-')}</div>
        </div>
        <div class="btn-group">
          <button type="button" class="btn btn-ghost btn-sm" data-session-details="${esc(item?.id || '')}">Details</button>
        </div>
      </div>
    `;
  }

  function renderRunnerSessionDetails(item) {
    const channels = Array.isArray(item?.channels) ? item.channels.join(', ') : '-';
    return `
      <div class="list">
        <div class="list-item">
          <div class="list-item-content">
            <div class="list-item-title">Label</div>
            <div class="list-item-description">${esc(item?.display_label || '-')}</div>
          </div>
        </div>
        <div class="list-item">
          <div class="list-item-content">
            <div class="list-item-title">Runner</div>
            <div class="list-item-description">${esc(item?.runner_label || item?.runner_kind || '-')}</div>
          </div>
        </div>
        <div class="list-item">
          <div class="list-item-content">
            <div class="list-item-title">Status</div>
            <div class="list-item-description">${esc(item?.status || '-')}</div>
          </div>
        </div>
        <div class="list-item">
          <div class="list-item-content">
            <div class="list-item-title">Created</div>
            <div class="list-item-description">${esc(formatTimestamp(item?.created_at))}</div>
          </div>
        </div>
        <div class="list-item">
          <div class="list-item-content">
            <div class="list-item-title">Last Active</div>
            <div class="list-item-description">${esc(formatTimestamp(item?.last_active_at))}</div>
          </div>
        </div>
        <div class="list-item">
          <div class="list-item-content">
            <div class="list-item-title">Channels</div>
            <div class="list-item-description">${esc(channels || '-')}</div>
          </div>
        </div>
        <div class="list-item">
          <div class="list-item-content">
            <div class="list-item-title">Telegram Thread</div>
            <div class="list-item-description mono">${esc(item?.telegram_thread_id || '-')}</div>
            <div class="form-hint mono">chat=${esc(item?.telegram_chat_id || '-')} anchor=${esc(item?.telegram_message_anchor_id || '-')}</div>
          </div>
        </div>
        <div class="list-item">
          <div class="list-item-content">
            <div class="list-item-title">Runner Session Key</div>
            <div class="list-item-description mono">${esc(item?.runner_session_key || '-')}</div>
          </div>
        </div>
        <div class="list-item">
          <div class="list-item-content">
            <div class="list-item-title">Session ID</div>
            <div class="list-item-description mono">${esc(item?.id || '-')}</div>
          </div>
        </div>
      </div>
    `;
  }

  function isTransientRunnersFetchError(err) {
    if (!err) return false;
    const name = String(err.name || '').toLowerCase();
    const message = String(err.message || '').toLowerCase();
    if (name === 'aborterror') return true;
    if (message.includes('networkerror when attempting to fetch resource')) return true;
    if (message.includes('failed to fetch')) return true;
    return false;
  }

  async function fetchRunnersFleet(options = {}) {
    const force = !!options.force;
    if (!force && Array.isArray(runnersFleetCache)) {
      return runnersFleetCache;
    }
    if (!force && runnersFleetRequest) {
      return runnersFleetRequest;
    }

    const url = force ? '/api/runners?force=true' : '/api/runners';
    const request = api(url).then((result) => {
      const items = Array.isArray(result?.items) ? result.items : [];
      runnersFleetCache = items;
      return items;
    });
    if (!force) {
      runnersFleetRequest = request;
    }
    try {
      return await request;
    } finally {
      if (!force && runnersFleetRequest === request) {
        runnersFleetRequest = null;
      }
    }
  }

  function buildRunnerSessionsUrl(cursor) {
    const params = new URLSearchParams();
    const runner = activeRunnerSessionsFilter();
    if (runner) params.set('runner', runner);
    if (runnerSessionsState.filters.channel && runnerSessionsState.filters.channel !== 'all') {
      params.set('channel', runnerSessionsState.filters.channel);
    }
    if (runnerSessionsState.filters.status && runnerSessionsState.filters.status !== 'all') {
      params.set('status', runnerSessionsState.filters.status);
    }
    if (runnerSessionsState.filters.activity && runnerSessionsState.filters.activity !== 'all') {
      params.set('activity', runnerSessionsState.filters.activity);
    }
    if (runnerSessionsState.filters.q.trim()) {
      params.set('q', runnerSessionsState.filters.q.trim());
    }
    params.set('limit', String(RUNNER_SESSIONS_DEFAULT_LIMIT));
    params.set('cursor', String(cursor || 0));
    return `/api/sessions?${params.toString()}`;
  }

  async function loadRunners(options = {}) {
    const container = document.getElementById('runners-grid');
    if (!container) return;
    const force = !!options.force;
    const requestId = ++runnersLoadRequestId;

    try {
      if (Array.isArray(runnersFleetCache) && !force) {
        renderRunnerCards(runnersFleetCache);
      } else {
        container.innerHTML = '<div class="loading"><div class="spinner"></div></div>';
      }

      const items = await fetchRunnersFleet({ force });
      if (requestId !== runnersLoadRequestId) return;
      if (state.currentPage !== 'runners') return;
      renderRunnerCards(items);
    } catch (err) {
      if (requestId !== runnersLoadRequestId || state.currentPage !== 'runners') return;
      if (isTransientRunnersFetchError(err)) {
        if (Array.isArray(runnersFleetCache)) {
          renderRunnerCards(runnersFleetCache);
        }
        return;
      }

      if (Array.isArray(runnersFleetCache) && runnersFleetCache.length) {
        renderRunnerCards(runnersFleetCache);
        toast('Showing cached runner metadata. Use Refresh when Agent Ruler is reachable.', 'warning', 4500);
        return;
      }

      const summaryEl = document.getElementById('runners-filter-summary');
      container.classList.remove('runners-grid-single');
      if (summaryEl) summaryEl.textContent = '';
      container.innerHTML = `
        <div class="alert alert-error">
          <span class="alert-icon">✕</span>
          <div class="alert-content">
            <div class="alert-title">Failed to Load Runners</div>
            <div class="alert-message">${esc(err.message)}</div>
          </div>
        </div>
      `;
    }
  }

  async function loadRunnerSessions(options = {}) {
    const listEl = document.getElementById('runner-sessions-list');
    const summaryEl = document.getElementById('runner-sessions-summary');
    const loadMoreBtn = document.getElementById('runner-sessions-load-more');
    if (!listEl || !summaryEl || !loadMoreBtn) return;

    const reset = options.reset !== false;
    const cursor = reset ? 0 : Number.parseInt(runnerSessionsState.nextCursor || '0', 10) || 0;
    const requestId = ++runnerSessionsLoadRequestId;

    if (reset) {
      runnerSessionsState.items = [];
      runnerSessionsState.total = 0;
      runnerSessionsState.hasMore = false;
      runnerSessionsState.nextCursor = null;
      listEl.innerHTML = '<div class="loading"><div class="spinner"></div></div>';
      summaryEl.textContent = 'Loading recent sessions...';
      loadMoreBtn.disabled = true;
    } else {
      loadMoreBtn.disabled = true;
      loadMoreBtn.textContent = 'Loading...';
    }

    try {
      const payload = await api(buildRunnerSessionsUrl(cursor));
      if (requestId !== runnerSessionsLoadRequestId || state.currentPage !== 'runners') return;

      const items = Array.isArray(payload?.items) ? payload.items : [];
      runnerSessionsState.items = reset ? items : runnerSessionsState.items.concat(items);
      runnerSessionsState.total = Number(payload?.total || 0);
      runnerSessionsState.hasMore = !!payload?.has_more;
      runnerSessionsState.nextCursor = payload?.next_cursor || null;
      renderRunnerSessionsList();
    } catch (err) {
      if (requestId !== runnerSessionsLoadRequestId || state.currentPage !== 'runners') return;
      summaryEl.textContent = '';
      listEl.innerHTML = `
        <div class="alert alert-error">
          <span class="alert-icon">✕</span>
          <div class="alert-content">
            <div class="alert-title">Failed to Load Sessions</div>
            <div class="alert-message">${esc(err.message)}</div>
          </div>
        </div>
      `;
      loadMoreBtn.disabled = true;
      loadMoreBtn.textContent = 'Load more';
    }
  }

  function renderRunnerSessionsList() {
    const listEl = document.getElementById('runner-sessions-list');
    const summaryEl = document.getElementById('runner-sessions-summary');
    const loadMoreBtn = document.getElementById('runner-sessions-load-more');
    if (!listEl || !summaryEl || !loadMoreBtn) return;

    const runner = activeRunnerSessionsFilter();
    const runnerLabel = RUNNERS_VIEW_OPTIONS.find((option) => option.id === runner)?.label || 'All runners';
    const shown = runnerSessionsState.items.length;
    const total = runnerSessionsState.total;
    const searchLabel = runnerSessionsState.filters.q.trim()
      ? ` Matching "${runnerSessionsState.filters.q.trim()}".`
      : '';
    summaryEl.textContent = `Showing ${shown} of ${total} ${runner ? `${runnerLabel} ` : ''}sessions. ${runnerSessionsState.filters.activity === 'recent' ? 'Recent activity only.' : 'All recorded activity.'}${searchLabel}`;

    if (!shown) {
      listEl.innerHTML = `
        <div class="empty-state">
          <div class="empty-state-icon">🧵</div>
          <div class="empty-state-title">No Sessions Yet</div>
          <div class="empty-state-description">No sessions match the current filters. Try a broader runner view or switch activity to all.</div>
        </div>
      `;
      loadMoreBtn.disabled = true;
      loadMoreBtn.textContent = 'Load more';
      return;
    }

    listEl.innerHTML = `<div class="list">${runnerSessionsState.items.map((item) => runnerSessionRow(item)).join('')}</div>`;
    Array.from(listEl.querySelectorAll('[data-session-details]')).forEach((button) => {
      button.addEventListener('click', async () => {
        const sessionId = String(button.getAttribute('data-session-details') || '').trim();
        if (!sessionId) return;
        try {
          const session = await api(`/api/sessions/${encodeURIComponent(sessionId)}`);
          openModal('Session Details', renderRunnerSessionDetails(session));
        } catch (err) {
          toast(`Failed to load session details: ${err.message}`, 'error');
        }
      });
    });

    loadMoreBtn.disabled = !runnerSessionsState.hasMore;
    loadMoreBtn.textContent = runnerSessionsState.hasMore ? 'Load more' : 'All loaded';
  }

  async function preloadRunnersFleet() {
    if (Array.isArray(runnersFleetCache) || runnersFleetRequest) return;
    try {
      await fetchRunnersFleet();
    } catch (_) {
      // Best-effort warm cache only.
    }
  }

  window.preloadRunnersFleet = preloadRunnersFleet;

  function renderRunnerCards(items) {
    const container = document.getElementById('runners-grid');
    const summaryEl = document.getElementById('runners-filter-summary');
    if (!container) return;

    const activeView = normalizeRunnersView(localStorage.getItem(RUNNERS_VIEW_STORAGE_KEY));
    updateRunnerTabState(activeView);

    if (!items.length) {
      if (summaryEl) summaryEl.textContent = 'No runner metadata is available for this runtime yet.';
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-state-icon">🏃</div>
          <div class="empty-state-title">No Runners Found</div>
          <div class="empty-state-description">No runner metadata is available for this runtime yet.</div>
        </div>
      `;
      return;
    }

    const filteredItems = activeView === 'all'
      ? items
      : items.filter((item) => String(item?.id || '').toLowerCase() === activeView);
    const shownCount = filteredItems.length;
    const totalCount = items.length;

    if (summaryEl) {
      const label = RUNNERS_VIEW_OPTIONS.find((option) => option.id === activeView)?.label || 'All';
      summaryEl.textContent = activeView === 'all'
        ? `Showing all runners (${shownCount}/${totalCount}).`
        : `Showing ${label} (${shownCount}/${totalCount}).`;
    }

    if (!shownCount) {
      container.innerHTML = `
        <div class="empty-state">
          <div class="empty-state-icon">🔎</div>
          <div class="empty-state-title">No Matching Runner</div>
          <div class="empty-state-description">The selected runner filter has no matching runtime metadata.</div>
        </div>
      `;
      return;
    }

    // Remove the grid classes to clear free-form blocks
    container.className = '';

    if (activeView === 'all') {
      container.innerHTML = `
        <div class="table-container" style="border: 1px solid var(--content-border); border-radius: var(--radius-lg); overflow-x: auto; max-height: 600px; overflow-y: auto; background: var(--content-bg);">
          <table class="table" style="margin: 0; min-width: 900px; border: none; width: 100%; border-collapse: collapse; text-align: left;">
            <thead style="position: sticky; top: 0; background: var(--content-bg-alt); z-index: 10; border-bottom: 1px solid var(--content-border);">
              <tr>
                <th style="padding: var(--space-3); font-weight: 600; color: var(--text-secondary);">&nbsp;Runner ID</th>
                <th style="padding: var(--space-3); font-weight: 600; color: var(--text-secondary);">Binary & Version</th>
                <th style="padding: var(--space-3); font-weight: 600; color: var(--text-secondary);">Health & Mode</th>
                <th style="padding: var(--space-3); font-weight: 600; color: var(--text-secondary);">Capabilities</th>
                <th style="padding: var(--space-3); font-weight: 600; color: var(--text-secondary);">Config Visibility</th>
              </tr>
            </thead>
            <tbody>
              ${filteredItems.map(item => {
                const installed = !!item.installed;
                const binary = item.binary || {};
                const health = item.health || {};
                const mode = item.mode || {};
                const capabilities = Array.isArray(item.capabilities) ? item.capabilities : [];
                const config = item.config || {};
                const runtime = state.runtime || {};
                
                return `
                  <tr style="border-bottom: 1px solid var(--content-border); background: var(--bg-primary);">
                    <td style="padding: var(--space-3); vertical-align: top;">
                      <div class="mono text-primary" style="font-weight: 600;">${esc(item.id || '-')}</div>
                      <div style="font-size: 0.8rem; margin-top: 6px;">
                        <span class="chip ${installed ? 'chip-success' : 'chip-danger'}">${installed ? 'installed' : 'missing'}</span>
                      </div>
                    </td>
                    <td style="padding: var(--space-3); vertical-align: top;">
                      <div class="mono" style="font-size: 0.85rem;">${esc(aliasRuntimePath(binary.path || binary.command || '-'))}</div>
                      <div style="font-size: 0.8rem; color: var(--text-muted); margin-top: 4px;">${esc(binary.version || 'v?')}</div>
                    </td>
                    <td style="padding: var(--space-3); vertical-align: top;">
                      <div><span style="color: var(--text-secondary);">status:</span> ${esc(health.status || '-')}</div>
                      <div style="font-size: 0.85rem; margin-top: 4px;"><span style="color: var(--text-secondary);">hs:</span> <span class="mono">${esc(health.handshake || '-')}</span></div>
                      <div style="font-size: 0.85rem; margin-top: 4px; color: var(--text-muted);">mode: ${esc(mode.current || '-')}</div>
                    </td>
                    <td style="padding: var(--space-3); vertical-align: top;">
                      <div style="display: flex; flex-wrap: wrap; gap: 4px; max-width: 200px;">
                        ${capabilities.length ? capabilities.map((cap) => `<span class="chip" style="font-size: 0.75rem; padding: 2px 6px;">${esc(cap)}</span>`).join('') : '<span class="text-muted" style="font-size: 0.85rem;">none</span>'}
                      </div>
                    </td>
                    <td style="padding: var(--space-3); vertical-align: top;">
                      <div style="display: flex; flex-direction: column; gap: 4px; font-size: 0.8rem;">
                        <div class="mono text-muted text-truncate" style="max-width: 250px;" title="${esc(aliasRuntimePath(config.managed_home || '-'))}">home: ${esc(aliasRuntimePath(config.managed_home || '-'))}</div>
                        <div class="mono text-muted text-truncate" style="max-width: 250px;" title="${esc(aliasRuntimePath(config.managed_workspace || '-'))}">work: ${esc(aliasRuntimePath(config.managed_workspace || '-'))}</div>
                        <div class="mono text-muted text-truncate" style="max-width: 250px;" title="${esc(aliasRuntimePath(runtime.shared_zone || '-'))}">shared: ${esc(aliasRuntimePath(runtime.shared_zone || '-'))}</div>
                      </div>
                    </td>
                  </tr>
                `;
              }).join('')}
            </tbody>
          </table>
        </div>
      `;
    } else {
      // Focus on selected runner
      container.innerHTML = `
        <div style="border: 1px solid var(--content-border); border-radius: var(--radius-lg); overflow: hidden; background: var(--content-bg); margin: 0 auto;">
          ${filteredItems.map(item => runnerCardFocused(item)).join('')}
        </div>
      `;
    }
  }

  function runnerCardFocused(item) {
    const installed = !!item.installed;
    const selected = !!item.selected;
    const configured = !!item.configured;
    const binary = item.binary || {};
    const health = item.health || {};
    const mode = item.mode || {};
    const warnings = Array.isArray(item.warnings) ? item.warnings : [];
    const capabilities = Array.isArray(item.capabilities) ? item.capabilities : [];
    const config = item.config || {};
    const masked = config.masked && typeof config.masked === 'object' ? config.masked : {};
    const maskedRows = Object.entries(masked);
    const integrations = Array.isArray(config.integrations) ? config.integrations : [];
    const runtime = state.runtime || {};
    const deliveryPath = runtime.default_user_destination_dir || runtime.default_delivery_dir || '-';

    return `
      <div style="padding: var(--space-4); border-bottom: 1px solid var(--content-border); background: var(--content-bg-alt); display: flex; justify-content: space-between; align-items: flex-start;">
        <div>
          <h4 style="font-size: 1.1rem; font-weight: 600; margin: 0 0 var(--space-1) 0; color: var(--text-primary);">${esc(item.label || item.id || 'runner')}</h4>
          <p style="font-size: 0.85rem; color: var(--text-muted); margin: 0;" class="mono">${esc(item.id || '')}</p>
        </div>
        <div class="approval-badges">
          ${selected ? '<span class="chip chip-primary">selected</span>' : ''}
          ${configured ? '<span class="chip">configured</span>' : '<span class="chip">not configured</span>'}
          <span class="chip ${installed ? 'chip-success' : 'chip-danger'}">${installed ? 'installed' : 'missing'}</span>
        </div>
      </div>
      <div style="padding: var(--space-4);">
        <div style="display: flex; flex-direction: column; gap: var(--space-3); font-size: 0.9rem;">
          <div style="display: grid; grid-template-columns: 180px 1fr; gap: var(--space-4); align-items: baseline; padding-bottom: var(--space-3); border-bottom: 1px solid var(--content-border);">
            <div style="color: var(--text-secondary); font-weight: 500;">Runner ID</div>
            <div class="mono text-primary">${esc(item.id || '-')}</div>
          </div>
          
          <div style="display: grid; grid-template-columns: 180px 1fr; gap: var(--space-4); align-items: baseline; padding-bottom: var(--space-3); border-bottom: 1px solid var(--content-border);">
            <div style="color: var(--text-secondary); font-weight: 500;">Binary</div>
            <div>
              <div class="mono text-primary">${esc(aliasRuntimePath(binary.path || binary.command || '-'))}</div>
              <div style="font-size: 0.8rem; color: var(--text-muted); margin-top: 4px;">${esc(binary.version || 'version unavailable')}</div>
            </div>
          </div>
          
          <div style="display: grid; grid-template-columns: 180px 1fr; gap: var(--space-4); align-items: baseline; padding-bottom: var(--space-3); border-bottom: 1px solid var(--content-border);">
            <div style="color: var(--text-secondary); font-weight: 500;">Health / Handshake</div>
            <div class="mono text-primary">${esc(health.status || '-')} <span style="color: var(--text-muted);">/</span> ${esc(health.handshake || '-')}</div>
          </div>
          
          <div style="display: grid; grid-template-columns: 180px 1fr; gap: var(--space-4); align-items: baseline; padding-bottom: var(--space-3); border-bottom: 1px solid var(--content-border);">
            <div style="color: var(--text-secondary); font-weight: 500;">Mode</div>
            <div>
              <div class="mono text-primary">${esc(mode.current || '-')}</div>
              <div style="font-size: 0.8rem; color: var(--text-muted); margin-top: 4px;">Supported: ${esc((mode.supported || []).join(', ') || '-')}</div>
            </div>
          </div>
          
          <div style="display: grid; grid-template-columns: 180px 1fr; gap: var(--space-4); align-items: baseline; padding-bottom: var(--space-3); border-bottom: 1px solid var(--content-border);">
            <div style="color: var(--text-secondary); font-weight: 500;">Capabilities</div>
            <div style="display: flex; flex-wrap: wrap; gap: 4px;">
              ${capabilities.length ? capabilities.map((cap) => `<span class="chip" style="font-size: 0.75rem; padding: 2px 6px;">${esc(cap)}</span>`).join('') : '<span class="text-muted">none</span>'}
            </div>
          </div>
          
          <div style="display: grid; grid-template-columns: 180px 1fr; gap: var(--space-4); align-items: baseline; padding-bottom: var(--space-3); border-bottom: 1px solid var(--content-border);">
            <div style="color: var(--text-secondary); font-weight: 500;">Managed Config</div>
            <div style="display: flex; flex-direction: column; gap: 4px; font-size: 0.85rem;">
              <div><span style="color: var(--text-muted); display: inline-block; width: 90px;">Home:</span> <span class="mono">${esc(aliasRuntimePath(config.managed_home || '-'))}</span></div>
              <div><span style="color: var(--text-muted); display: inline-block; width: 90px;">Workspace:</span> <span class="mono">${esc(aliasRuntimePath(config.managed_workspace || '-'))}</span></div>
              <div><span style="color: var(--text-muted); display: inline-block; width: 90px;">Integrations:</span> ${esc(integrations.join(', ') || 'none')}</div>
              ${maskedRows.length ? `<div style="margin-top: 8px;"><div style="color: var(--text-muted); margin-bottom: 4px;">Masked keys:</div> ${maskedRows.map(([key, value]) => `<div class="mono" style="padding-left: 10px; border-left: 2px solid var(--content-border);">${esc(key)}=${esc(String(value))}</div>`).join('')}</div>` : ''}
            </div>
          </div>
          
          <div style="display: grid; grid-template-columns: 180px 1fr; gap: var(--space-4); align-items: baseline; padding-bottom: var(--space-3); ${warnings.length ? 'border-bottom: 1px solid var(--content-border);' : ''}">
            <div style="color: var(--text-secondary); font-weight: 500;">Zone Visibility</div>
            <div style="display: flex; flex-direction: column; gap: 4px; font-size: 0.85rem;">
              <div><span style="color: var(--text-muted); display: inline-block; width: 140px;">Zone 0 (workspace):</span> <span class="mono">${esc(aliasRuntimePath(runtime.workspace || '-'))}</span></div>
              <div><span style="color: var(--text-muted); display: inline-block; width: 140px;">Zone 2 (shared):</span> <span class="mono">${esc(aliasRuntimePath(runtime.shared_zone || '-'))}</span></div>
              <div><span style="color: var(--text-muted); display: inline-block; width: 140px;">Zone 1 (delivery):</span> <span class="mono">${esc(aliasRuntimePath(deliveryPath))}</span></div>
            </div>
          </div>
          
          ${warnings.length ? `
            <div style="display: grid; grid-template-columns: 180px 1fr; gap: var(--space-4); align-items: baseline; padding-bottom: var(--space-3); padding-top: var(--space-3); background: color-mix(in srgb, var(--danger) 5%, transparent); margin: 0 -var(--space-4) -var(--space-4) -var(--space-4); padding-left: var(--space-4); padding-right: var(--space-4);">
              <div style="color: var(--danger); font-weight: 600;">Warnings</div>
              <div style="display: flex; flex-direction: column; gap: 4px; color: var(--danger); font-size: 0.85rem;">
                ${warnings.map((w) => `<div style="display: flex; gap: 6px;"><span>⚠</span> <span>${esc(w)}</span></div>`).join('')}
              </div>
            </div>
          ` : ''}
        </div>
      </div>
    `;
  }
