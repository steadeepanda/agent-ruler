  // ============================================
  // Status & Sidebar
  // ============================================

  const UPDATE_CHECK_CACHE_KEY = 'ar.update.check.cache.v1';
  const UPDATE_CHECK_NOTIFIED_TAG_KEY = 'ar.update.notified_tag';
  const UPDATE_CHECK_INTERVAL_MS = 2 * 60 * 60 * 1000;
  const RUNNER_LABELS = {
    openclaw: 'OpenClaw',
    claudecode: 'Claude Code',
    opencode: 'OpenCode'
  };

  function selectedRunnerId() {
    const runtimeRunner = String(state.runtime?.selected_runner || '').trim().toLowerCase();
    if (runtimeRunner) return runtimeRunner;
    return String(state.status?.selected_runner || '').trim().toLowerCase();
  }

  function selectedRunnerLabel() {
    const runnerId = selectedRunnerId();
    if (!runnerId) return 'Not selected';
    return RUNNER_LABELS[runnerId] || runnerId;
  }

  function activeBridgeRunnerId() {
    const runtimeRunner = String(state.runtime?.telegram_bridge_active_runner || '').trim().toLowerCase();
    if (runtimeRunner) return runtimeRunner;
    return String(state.status?.telegram_bridge_active_runner || '').trim().toLowerCase();
  }

  function activeBridgeRunnerLabel() {
    const runnerId = activeBridgeRunnerId();
    if (!runnerId) return 'None';
    return RUNNER_LABELS[runnerId] || runnerId;
  }

  function telegramBridgeInSync() {
    if (typeof state.runtime?.telegram_bridge_in_sync === 'boolean') {
      return !!state.runtime.telegram_bridge_in_sync;
    }
    if (typeof state.status?.telegram_bridge_in_sync === 'boolean') {
      return !!state.status.telegram_bridge_in_sync;
    }
    return true;
  }

  async function refreshStatus() {
    try {
      const [status, runtime] = await Promise.all([
        api('/api/status'),
        api('/api/runtime')
      ]);
      
      state.status = status;
      state.runtime = runtime;
      
      updateHeader();
      updateSidebarInfo();
      
    } catch (err) {
      console.error('Failed to refresh status:', err);
    }
  }

  function updateHeader() {
    const s = state.status;
    if (!s) return;
    
    const title = document.querySelector('.header-title');
    if (title) {
      const labels = {
        overview: 'Overview',
        approvals: 'Approvals',
        'approval-detail': 'Approval Details',
        files: 'Files',
        policy: 'Policy',
        receipts: 'Receipts',
        runners: 'Runners',
        runtime: 'Runtime Paths',
        settings: 'Control Settings',
        execution: 'Execution Layer'
      };
      title.textContent = labels[state.currentPage] || 'Agent Ruler';
    }
    
    const chips = document.querySelector('.header-chips');
    if (chips) {
      const updateChip = state.update && state.update.update_available
        ? `<a href="/settings" class="chip chip-warning">Update: ${esc(state.update.latest_tag || 'available')}</a>`
        : '';
      const runnerChip = `<span class="chip chip-success">Runner: ${esc(selectedRunnerLabel())}</span>`;
      const bridgeChipClass = telegramBridgeInSync() ? 'chip' : 'chip chip-warning';
      const bridgeChip = `<span class="${bridgeChipClass}">Telegram Bridge: ${esc(activeBridgeRunnerLabel())}</span>`;
      chips.innerHTML = `
        <span class="chip">Version: v${esc(s.app_version || '0.0.0')}</span>
        ${runnerChip}
        ${bridgeChip}
        ${updateChip}
        <span class="chip">Profile: ${esc(s.profile)}</span>
        <span class="chip chip-primary">Pending: ${s.pending_approvals}</span>
      `;
    }
  }

  function updateSidebarInfo() {
    const r = state.runtime;
    if (!r) return;
    
    const info = document.querySelector('.sidebar-info-content');
    if (info) {
      const updateRow = state.update && state.update.update_available
        ? `<div class="runtime-info-row"><strong>Update:</strong> ${esc(state.update.latest_tag || 'available')}</div>`
        : '';
      info.innerHTML = `
        <div class="runtime-info-row"><strong>Version:</strong> v${esc(r.app_version || state.status?.app_version || '0.0.0')}</div>
        ${updateRow}
        <div class="runtime-info-row"><strong>Selected Runner:</strong> ${esc(selectedRunnerLabel())}</div>
        <div class="runtime-info-row"><strong>Telegram Bridge:</strong> ${esc(activeBridgeRunnerLabel())}${telegramBridgeInSync() ? '' : ' (out of sync)'}</div>
        <div class="runtime-info-row"><strong>Bind:</strong> ${esc(state.config?.ui_bind || state.runtime?.ui_bind || state.status?.ui_bind || 'n/a')}</div>
        <div class="runtime-info-row"><strong>Workspace:</strong></div>
        <div class="runtime-info-path">${esc(aliasRuntimePath(r.workspace))}</div>
        <div class="runtime-info-row"><strong>Shared Zone:</strong></div>
        <div class="runtime-info-path">${esc(aliasRuntimePath(r.shared_zone))}</div>
      `;
    }
    
    // Update approval badge
    const badge = document.querySelector('[data-badge="approvals"]');
    if (badge) {
      const count = state.status?.pending_approvals || 0;
      badge.textContent = count;
      badge.style.display = count > 0 ? 'inline-flex' : 'none';
    }
  }

  function readCachedUpdateCheckEnvelope() {
    try {
      const raw = localStorage.getItem(UPDATE_CHECK_CACHE_KEY);
      if (!raw) return null;
      const parsed = JSON.parse(raw);
      const checkedAt = Number(parsed.checked_at_ms || 0);
      if (!Number.isFinite(checkedAt) || checkedAt <= 0) return null;
      if ((Date.now() - checkedAt) > UPDATE_CHECK_INTERVAL_MS) return null;
      return parsed;
    } catch (_) {
      return null;
    }
  }

  function writeCachedUpdateStatus(payload, errorMessage) {
    try {
      localStorage.setItem(UPDATE_CHECK_CACHE_KEY, JSON.stringify({
        checked_at_ms: Date.now(),
        payload: payload && typeof payload === 'object' ? payload : null,
        error: errorMessage ? String(errorMessage) : null
      }));
    } catch (_) {}
  }

  function markUpdateNotified(tag) {
    if (!tag) return;
    try {
      localStorage.setItem(UPDATE_CHECK_NOTIFIED_TAG_KEY, tag);
    } catch (_) {}
  }

  function wasUpdateAlreadyNotified(tag) {
    if (!tag) return false;
    try {
      return localStorage.getItem(UPDATE_CHECK_NOTIFIED_TAG_KEY) === tag;
    } catch (_) {
      return false;
    }
  }

  function normalizeUpdatePayload(response) {
    if (!response || typeof response !== 'object') return null;
    const check = response.check || response.result?.check || response.result || null;
    if (!check || typeof check !== 'object') return null;
    return check;
  }

  async function fetchUpdateStatus(options = {}) {
    const force = !!options.force;
    const quiet = !!options.quiet;

    if (!force) {
      const cachedEnvelope = readCachedUpdateCheckEnvelope();
      const cachedPayload = cachedEnvelope && cachedEnvelope.payload && typeof cachedEnvelope.payload === 'object'
        ? cachedEnvelope.payload
        : null;
      if (cachedEnvelope) {
        if (cachedPayload) {
          state.update = cachedPayload;
          updateHeader();
          updateSidebarInfo();
          return cachedPayload;
        }
        // Recent failed/empty check attempt: do not hammer GitHub on every poll.
        return state.update;
      }
    }

    let payload;
    try {
      const response = await api('/api/update/check');
      payload = normalizeUpdatePayload(response);
    } catch (err) {
      writeCachedUpdateStatus(null, err.message || String(err));
      if (!quiet) {
        toast(`Update check failed: ${err.message}`, 'warning');
        recordUiEvent('warning', 'update-check', `Update check failed: ${err.message}`);
      }
      return state.update;
    }

    if (!payload) {
      writeCachedUpdateStatus(null, 'unexpected_response');
      if (!quiet) {
        toast('Update check returned an unexpected response', 'warning');
        recordUiEvent('warning', 'update-check', 'Update check returned an unexpected response');
      }
      return state.update;
    }

    state.update = payload;
    writeCachedUpdateStatus(payload, null);
    updateHeader();
    updateSidebarInfo();

    if (payload.update_available) {
      const latestTag = String(payload.latest_tag || '').trim();
      if (!wasUpdateAlreadyNotified(latestTag)) {
        toast(`New update available: ${latestTag}`, 'info', 7000, {
          linkHref: '/settings',
          linkLabel: 'Update now'
        });
        recordUiEvent('info', 'update-check', `New update available: ${latestTag}`, {
          latest_tag: latestTag,
          current_version: payload.current_version || null
        });
        markUpdateNotified(latestTag);
      } else if (!quiet) {
        toast(`Update available: ${latestTag}`, 'info', 5000, {
          linkHref: '/settings',
          linkLabel: 'Open settings'
        });
        recordUiEvent('info', 'update-check', `Update available: ${latestTag}`);
      }
    } else if (!quiet) {
      toast(`Already up to date (v${payload.current_version || 'unknown'})`, 'success');
      recordUiEvent('info', 'update-check', `Already up to date (v${payload.current_version || 'unknown'})`);
    }

    return payload;
  }

  // ============================================
  // Initialization
  // ============================================

  // ============================================
  // Approval Detail Page (Deep Link)
  // ============================================

  async function renderApprovalDetail(root) {
    // Extract approval ID from URL path
    const pathParts = window.location.pathname.split('/').filter(Boolean);
    const approvalId = decodeURIComponent(pathParts[pathParts.length - 1] || '');
    
    root.innerHTML = `
      <div class="card">
        <div class="card-header">
          <div>
            <h3 class="card-title">Approval Details</h3>
            <p class="card-description mono">${esc(approvalId)}</p>
          </div>
          <a href="/approvals" class="btn btn-ghost btn-sm">← Back to List</a>
        </div>
        <div class="card-body">
          <div id="approval-detail-content">
            <div class="loading">Loading approval details...</div>
          </div>
        </div>
      </div>
    `;
    
    try {
      const data = await api(`/api/approvals/${approvalId}`);
      renderApprovalDetailContent(data);
    } catch (err) {
      const container = document.getElementById('approval-detail-content');
      if (container) {
        container.innerHTML = `
          <div class="alert alert-error">
            <span class="alert-icon">✕</span>
            <div class="alert-content">
              <div class="alert-title">Failed to Load Approval</div>
              <div class="alert-message">${esc(err.message)}</div>
            </div>
          </div>
        `;
      }
    }
  }

  function renderApprovalDetailContent(item) {
    const container = document.getElementById('approval-detail-content');
    if (!container) return;
    const approval = item?.approval || item || {};
    const approvalId = approval.id || '';
    const canAct = approval.status === 'Pending' && !!approvalId;
    
    const statusChip = approval.status === 'Pending'
      ? '<span class="chip chip-warning">Pending</span>'
      : approval.status === 'Approved'
      ? '<span class="chip chip-success">Approved</span>'
      : '<span class="chip chip-danger">Denied</span>';
    
    container.innerHTML = `
      <div class="approval-detail">
        <div class="approval-detail-header">
          <div class="approval-badges">
            ${statusChip}
            <span class="chip">${esc(approval.reason)}</span>
            <span class="chip">${esc(approval.action?.metadata?.runner_id || 'unknown')}</span>
          </div>
        </div>
        
        <div class="detail-section">
          <h4>Why This Was Flagged</h4>
          <p class="why-explanation">${esc(item.why)}</p>
        </div>
        
        <div class="detail-section">
          <h4>Request Details</h4>
          <div class="detail-grid">
            <div class="detail-item">
              <span class="detail-label">Operation</span>
              <span class="detail-value">${esc(approval.action?.operation)}</span>
            </div>
            <div class="detail-item">
              <span class="detail-label">Runner</span>
              <span class="detail-value">${esc(approval.action?.metadata?.runner_id || '-')}</span>
            </div>
            <div class="detail-item">
              <span class="detail-label">Source</span>
              <span class="detail-value mono">${esc(aliasRuntimePath(item.resolved_src || '-'))}</span>
            </div>
            <div class="detail-item">
              <span class="detail-label">Destination</span>
              <span class="detail-value mono">${esc(aliasRuntimePath(item.resolved_dst || '-'))}</span>
            </div>
            <div class="detail-item">
              <span class="detail-label">Created</span>
              <span class="detail-value">${formatRelativeTime(approval.created_at)}</span>
            </div>
            <div class="detail-item">
              <span class="detail-label">Expires</span>
              <span class="detail-value">${formatRelativeTime(approval.expires_at)}</span>
            </div>
          </div>
        </div>
        
        ${item.diff_summary ? `
          <div class="detail-section">
            <h4>Changes Preview</h4>
            <div class="diff-preview">${formatDiff(item.diff_summary)}</div>
          </div>
        ` : ''}
        
        ${canAct ? `
          <div class="detail-actions">
            <button class="btn btn-success" id="detail-approve">Approve</button>
            <button class="btn btn-danger" id="detail-deny">Deny</button>
          </div>
        ` : ''}
      </div>
    `;
    
    // Bind action buttons
    const approveBtn = document.getElementById('detail-approve');
    const denyBtn = document.getElementById('detail-deny');
    
    if (approveBtn) {
      approveBtn.addEventListener('click', async () => {
        try {
          await api(`/api/approvals/${approvalId}/approve`, { method: 'POST' });
          toast('Approved successfully', 'success');
          // Re-render with updated status
          const data = await api(`/api/approvals/${approvalId}`);
          renderApprovalDetailContent(data);
        } catch (err) {
          toast(`Failed to approve: ${err.message}`, 'error');
        }
      });
    }
    
    if (denyBtn) {
      denyBtn.addEventListener('click', async () => {
        try {
          await api(`/api/approvals/${approvalId}/deny`, { method: 'POST' });
          toast('Denied successfully', 'success');
          const data = await api(`/api/approvals/${approvalId}`);
          renderApprovalDetailContent(data);
        } catch (err) {
          toast(`Failed to deny: ${err.message}`, 'error');
        }
      });
    }
  }

  // ============================================
  // Browser Notifications
  // ============================================

  let lastPendingCount = 0;

  async function requestNotificationPermission() {
    if (!('Notification' in window)) {
      console.log('Browser does not support notifications');
      return false;
    }
    
    if (Notification.permission === 'granted') {
      return true;
    }
    
    if (Notification.permission !== 'denied') {
      const permission = await Notification.requestPermission();
      return permission === 'granted';
    }
    
    return false;
  }

  function notifyNewApprovals(count) {
    if (Notification.permission !== 'granted') return;
    
    const title = `Agent Ruler: ${count} New Approval${count > 1 ? 's' : ''}`;
    const options = {
      body: `You have ${count} pending approval${count > 1 ? 's' : ''} requiring your review.`,
      tag: 'agent-ruler-approvals',
      requireInteraction: false,
      actions: [
        { action: 'view', title: 'View Approvals' },
        { action: 'dismiss', title: 'Dismiss' }
      ]
    };
    
    try {
      const notification = new Notification(title, options);
      notification.onclick = () => {
        window.focus();
        window.location.href = '/approvals';
        notification.close();
      };
    } catch (err) {
      console.error('Failed to show notification:', err);
    }
  }

  function checkForNewApprovals(newCount) {
    if (newCount > lastPendingCount) {
      const newItems = newCount - lastPendingCount;
      toast(
        `${newItems} new approval${newItems > 1 ? 's' : ''} waiting for review`,
        'warning',
        5000
      );
      if (document.visibilityState === 'hidden') {
        notifyNewApprovals(newItems);
      }
    }
    lastPendingCount = newCount;
  }

  const SIDEBAR_COLLAPSE_STORAGE_KEY = 'ar.ui.sidebar.collapsed';

  function readSidebarCollapsedPreference() {
    return localStorage.getItem(SIDEBAR_COLLAPSE_STORAGE_KEY) === '1';
  }

  function setSidebarCollapsedPreference(collapsed) {
    localStorage.setItem(SIDEBAR_COLLAPSE_STORAGE_KEY, collapsed ? '1' : '0');
  }

  function applySidebarCollapsed(collapsed) {
    const body = document.body;
    if (!body) return;
    body.classList.toggle('sidebar-collapsed', !!collapsed);

    const btnLabel = collapsed ? 'Expand navigation' : 'Collapse navigation';
    ['sidebar-collapse-global-btn'].forEach((id) => {
      const btn = document.getElementById(id);
      if (!btn) return;
      btn.setAttribute('aria-label', btnLabel);
      btn.setAttribute('title', btnLabel);
      btn.textContent = collapsed ? '»' : '«';
    });
  }

  function bindSidebarCollapse() {
    const globalBtn = document.getElementById('sidebar-collapse-global-btn');
    if (!globalBtn) return;

    applySidebarCollapsed(readSidebarCollapsedPreference());
    const toggle = () => {
      const next = !document.body.classList.contains('sidebar-collapsed');
      setSidebarCollapsedPreference(next);
      applySidebarCollapsed(next);
    };

    globalBtn.addEventListener('click', toggle);
  }

  function bindMobileSidebar() {
    const sidebar = document.getElementById('sidebar');
    const overlay = document.getElementById('sidebar-overlay');
    const menuBtn = document.getElementById('mobile-menu-btn');
    if (!sidebar || !overlay || !menuBtn) return;

    const setOpen = (open) => {
      sidebar.classList.toggle('open', open);
      overlay.classList.toggle('active', open);
      menuBtn.setAttribute('aria-expanded', open ? 'true' : 'false');
    };

    menuBtn.addEventListener('click', () => {
      setOpen(!sidebar.classList.contains('open'));
    });

    overlay.addEventListener('click', () => setOpen(false));
    document.querySelectorAll('.sidebar .nav-item').forEach((link) => {
      link.addEventListener('click', () => setOpen(false));
    });
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') setOpen(false);
    });
    window.addEventListener('resize', () => {
      if (window.innerWidth > 1024) setOpen(false);
    });
  }
