  const RUNNERS_VIEW_STORAGE_KEY = 'ar.runners.view';
  const RUNNERS_VIEW_OPTIONS = [
    { id: 'all', label: 'All' },
    { id: 'openclaw', label: 'OpenClaw' },
    { id: 'claudecode', label: 'Claude Code' },
    { id: 'opencode', label: 'OpenCode' }
  ];
  const RUNNER_SESSIONS_DEFAULT_LIMIT = 6;
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
      <div class="card">
        <div class="card-header">
          <div>
            <h3 class="card-title">Runner Fleet</h3>
            <p class="card-description">
              Installed status, health handshakes, capabilities, and managed config visibility per runner.
            </p>
          </div>
          <button id="runners-refresh" class="btn btn-ghost btn-sm" type="button">Refresh</button>
        </div>
        <div class="card-body">
          <div class="panel-tabs" id="runners-tab-list" role="tablist" aria-label="Runner fleet filters">
            ${RUNNERS_VIEW_OPTIONS.map((option) => `
              <button type="button" class="panel-tab" data-runners-tab="${esc(option.id)}" role="tab">${esc(option.label)}</button>
            `).join('')}
          </div>
          <p id="runners-filter-summary" class="form-hint mb-4"></p>
          <div id="runners-grid" class="grid grid-2"></div>
        </div>
      </div>

      <div class="card mt-5">
        <div class="card-header">
          <div>
            <h3 class="card-title">Recent Sessions</h3>
            <p class="card-description">
              Find runner-bound sessions without loading the full history up front. Search first, then load more only when needed.
            </p>
          </div>
        </div>
        <div class="card-body">
          <div class="grid grid-4">
            <div class="form-group">
              <label class="form-label" for="runner-sessions-search">Search</label>
              <input id="runner-sessions-search" class="form-input" type="search" placeholder="Session id, label, thread, runner key" value="${esc(runnerSessionsState.filters.q)}" />
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
          <div id="runner-sessions-list"></div>
          <div class="mt-4">
            <button id="runner-sessions-load-more" class="btn btn-ghost btn-sm" type="button">Load more</button>
          </div>
        </div>
      </div>
    `;

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

  function runnerCard(item) {
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
      <div class="card">
        <div class="card-header">
          <div>
            <h4 class="card-title">${esc(item.label || item.id || 'runner')}</h4>
            <p class="card-description mono">${esc(item.id || '')}</p>
          </div>
          <div class="approval-badges">
            ${selected ? '<span class="chip chip-primary">selected</span>' : ''}
            ${configured ? '<span class="chip">configured</span>' : '<span class="chip">not configured</span>'}
            <span class="chip ${installed ? 'chip-success' : 'chip-danger'}">${installed ? 'installed' : 'missing'}</span>
          </div>
        </div>
        <div class="card-body">
          <div class="list">
            <div class="list-item">
              <div class="list-item-content">
                <div class="list-item-title">Runner ID</div>
                <div class="list-item-description mono">${esc(item.id || '-')}</div>
              </div>
            </div>
            <div class="list-item">
              <div class="list-item-content">
                <div class="list-item-title">Binary</div>
                <div class="list-item-description mono">${esc(aliasRuntimePath(binary.path || binary.command || '-'))}</div>
                <div class="form-hint">${esc(binary.version || 'version unavailable')}</div>
              </div>
            </div>
            <div class="list-item">
              <div class="list-item-content">
                <div class="list-item-title">Health / Handshake</div>
                <div class="list-item-description mono">${esc(health.status || '-')} / ${esc(health.handshake || '-')}</div>
              </div>
            </div>
            <div class="list-item">
              <div class="list-item-content">
                <div class="list-item-title">Mode</div>
                <div class="list-item-description mono">${esc(mode.current || '-')}</div>
                <div class="form-hint">Supported: ${esc((mode.supported || []).join(', ') || '-')}</div>
              </div>
            </div>
            <div class="list-item">
              <div class="list-item-content">
                <div class="list-item-title">Capabilities (reported)</div>
                <div class="list-item-description">${capabilities.length ? capabilities.map((cap) => `<span class="chip">${esc(cap)}</span>`).join(' ') : '<span class="text-muted">none</span>'}</div>
              </div>
            </div>
            <div class="list-item">
              <div class="list-item-content">
                <div class="list-item-title">Managed Config</div>
                <div class="form-hint">Home: <span class="mono">${esc(aliasRuntimePath(config.managed_home || '-'))}</span></div>
                <div class="form-hint">Workspace: <span class="mono">${esc(aliasRuntimePath(config.managed_workspace || '-'))}</span></div>
                <div class="form-hint">Integrations: ${esc(integrations.join(', ') || 'none')}</div>
                ${maskedRows.length ? `<div class="form-hint">Masked keys: ${maskedRows.map(([key, value]) => `${esc(key)}=${esc(String(value))}`).join(', ')}</div>` : ''}
              </div>
            </div>
            <div class="list-item">
              <div class="list-item-content">
                <div class="list-item-title">Zone Visibility</div>
                <div class="form-hint">Zone 0 (workspace): <span class="mono">${esc(aliasRuntimePath(runtime.workspace || '-'))}</span></div>
                <div class="form-hint">Zone 2 (shared): <span class="mono">${esc(aliasRuntimePath(runtime.shared_zone || '-'))}</span></div>
                <div class="form-hint">Zone 1 (delivery): <span class="mono">${esc(aliasRuntimePath(deliveryPath))}</span></div>
              </div>
            </div>
            ${warnings.length ? `
              <div class="list-item">
                <div class="list-item-content">
                  <div class="list-item-title">Warnings</div>
                  <div class="list-item-description">
                    ${warnings.map((warning) => `<div class="form-hint">${esc(warning)}</div>`).join('')}
                  </div>
                </div>
              </div>
            ` : ''}
          </div>
        </div>
      </div>
    `;
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
      container.classList.remove('runners-grid-single');
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

    container.classList.toggle('runners-grid-single', shownCount <= 1);
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

    container.innerHTML = filteredItems.map((item) => runnerCard(item)).join('');
  }
