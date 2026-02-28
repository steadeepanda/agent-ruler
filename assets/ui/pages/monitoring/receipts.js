  // Extracted from assets/ui/20-pages-config.js for monitoring-scoped editing.
  function renderReceipts(root) {
    const receiptFilters = state.receipts.filters;
    const logFilters = state.logs.filters;
    const showDetails = !!state.receipts.showDetails;
    const mode = state.receipts.mode === 'logs' ? 'logs' : 'receipts';

    root.innerHTML = `
      <div class="card">
        <div class="card-header">
          <div>
            <h3 class="card-title">Timeline</h3>
            <p class="card-description">Switch between governed action receipts and Control Panel logs <span class="chip chip-success" style="font-size: 0.75rem; margin-left: 8px;">● Live</span></p>
          </div>
        </div>
        <div class="card-body">
          <div class="timeline-mode-switch mb-4">
            <button id="timeline-mode-receipts" class="btn btn-sm ${mode === 'receipts' ? 'btn-primary' : 'btn-ghost'}" type="button">Receipts</button>
            <button id="timeline-mode-logs" class="btn btn-sm ${mode === 'logs' ? 'btn-primary' : 'btn-ghost'}" type="button">Logs</button>
          </div>
          <div class="filters">
            ${mode === 'receipts' ? `
              <div class="filter-group">
                <label class="filter-label">Date</label>
                <input type="date" id="filter-date" class="form-input" value="${esc(receiptFilters.date)}" />
              </div>
              <div class="filter-group">
                <label class="filter-label">Verdict</label>
                <select id="filter-verdict" class="form-select">
                  <option value="">All</option>
                  <option value="allow" ${receiptFilters.verdict === 'allow' ? 'selected' : ''}>Allow</option>
                  <option value="deny" ${receiptFilters.verdict === 'deny' ? 'selected' : ''}>Deny</option>
                  <option value="require_approval" ${receiptFilters.verdict === 'require_approval' ? 'selected' : ''}>Approval</option>
                  <option value="quarantine" ${receiptFilters.verdict === 'quarantine' ? 'selected' : ''}>Quarantine</option>
                </select>
              </div>
              <div class="filter-group">
                <label class="filter-label">Search</label>
                <input type="text" id="filter-q" class="form-input" placeholder="Search..." value="${esc(receiptFilters.q)}" />
              </div>
            ` : `
              <div class="filter-group">
                <label class="filter-label">Level</label>
                <select id="log-filter-level" class="form-select">
                  <option value="">All</option>
                  <option value="error" ${logFilters.level === 'error' ? 'selected' : ''}>Error</option>
                  <option value="warning" ${logFilters.level === 'warning' ? 'selected' : ''}>Warning</option>
                  <option value="info" ${logFilters.level === 'info' ? 'selected' : ''}>Info</option>
                </select>
              </div>
              <div class="filter-group">
                <label class="filter-label">Source</label>
                <input type="text" id="log-filter-source" class="form-input" placeholder="update-check, update-apply..." value="${esc(logFilters.source)}" />
              </div>
              <div class="filter-group">
                <label class="filter-label">Search</label>
                <input type="text" id="log-filter-q" class="form-input" placeholder="Search..." value="${esc(logFilters.q)}" />
              </div>
            `}
            <div class="filter-actions">
              <button id="filter-apply" class="btn btn-primary btn-sm">Apply</button>
              <button id="filter-clear" class="btn btn-ghost btn-sm">Clear</button>
            </div>
          </div>
          ${mode === 'receipts' ? `
            <label class="form-check mb-4">
              <input type="checkbox" id="filter-show-details" class="form-check-input" ${showDetails ? 'checked' : ''} />
              <span class="form-check-label">Show operator-only debug details</span>
            </label>
            <label class="form-check mb-2">
              <input type="checkbox" id="filter-runtime-aliases" class="form-check-input" ${state.receipts.useRuntimeAliases ? 'checked' : ''} />
              <span class="form-check-label">Use runtime path labels (recommended)</span>
            </label>
            <p class="form-hint mb-4">Summary view keeps output readable. Debug mode shows full command/diff context. Runtime labels shorten long paths, for example <code class="mono">WORKSPACE_PATH/src/main.rs</code>.</p>
          ` : `
            <p class="form-hint mb-4">Logs mode captures Control Panel events (errors, warnings, update checks/apply lifecycle) to help retrace issues.</p>
          `}
          
          <div id="receipts-list" class="timeline"></div>
          
          <div id="receipts-pagination" class="pagination"></div>
        </div>
      </div>
    `;

    document.getElementById('timeline-mode-receipts').addEventListener('click', () => {
      if (state.receipts.mode === 'receipts') return;
      setTimelineMode('receipts');
      state.receipts.offset = 0;
      renderReceipts(root);
    });

    document.getElementById('timeline-mode-logs').addEventListener('click', () => {
      if (state.receipts.mode === 'logs') return;
      setTimelineMode('logs');
      state.logs.offset = 0;
      renderReceipts(root);
    });
    
    document.getElementById('filter-apply').addEventListener('click', () => {
      if (mode === 'receipts') {
        state.receipts.filters.date = document.getElementById('filter-date').value;
        state.receipts.filters.verdict = document.getElementById('filter-verdict').value;
        state.receipts.filters.q = document.getElementById('filter-q').value;
        state.receipts.offset = 0;
        loadReceipts();
        return;
      }

      state.logs.filters.level = document.getElementById('log-filter-level').value;
      state.logs.filters.source = document.getElementById('log-filter-source').value;
      state.logs.filters.q = document.getElementById('log-filter-q').value;
      state.logs.offset = 0;
      loadUiLogs();
    });

    document.getElementById('filter-clear').addEventListener('click', () => {
      if (mode === 'receipts') {
        state.receipts.filters.date = '';
        state.receipts.filters.verdict = '';
        state.receipts.filters.q = '';
        state.receipts.offset = 0;
        document.getElementById('filter-date').value = state.receipts.filters.date;
        document.getElementById('filter-verdict').value = '';
        document.getElementById('filter-q').value = '';
        loadReceipts();
        return;
      }

      state.logs.filters.level = '';
      state.logs.filters.source = '';
      state.logs.filters.q = '';
      state.logs.offset = 0;
      document.getElementById('log-filter-level').value = '';
      document.getElementById('log-filter-source').value = '';
      document.getElementById('log-filter-q').value = '';
      loadUiLogs();
    });

    if (mode === 'receipts') {
      document.getElementById('filter-show-details').addEventListener('change', (event) => {
        setReceiptDetailVisibility(!!event.target.checked);
        loadReceipts();
      });

      document.getElementById('filter-runtime-aliases').addEventListener('change', (event) => {
        setReceiptRuntimeAliasVisibility(!!event.target.checked);
        loadReceipts();
      });
    }
    
    if (mode === 'receipts') {
      loadReceipts();
    } else {
      loadUiLogs();
    }
  }

  async function loadReceipts() {
    const container = document.getElementById('receipts-list');
    if (!container) return;
    const showDetails = !!state.receipts.showDetails;
    
    const { limit, offset, filters } = state.receipts;
    const params = new URLSearchParams();
    params.set('limit', limit);
    params.set('offset', offset);
    if (showDetails) params.set('include_details', 'true');
    if (filters.date) params.set('date', filters.date);
    if (filters.verdict) params.set('verdict', filters.verdict);
    if (filters.q) params.set('q', filters.q);
    
    try {
      container.innerHTML = '<div class="loading"><div class="spinner"></div></div>';
      
      const result = await api(`/api/receipts?${params}`);
      state.receipts.items = result.items;
      state.receipts.total = result.total;
      state.receipts.hasMore = result.has_more;
      
      if (!result.items.length) {
        container.innerHTML = `
          <div class="empty-state">
            <div class="empty-state-icon">📜</div>
            <div class="empty-state-title">No Receipts</div>
            <div class="empty-state-description">No timeline entries match your current filters.</div>
          </div>
        `;
        return;
      }

      const parseDetailTargets = (detail) => {
        const matches = String(detail || '')
          .split('\n')
          .map((line) => line.trim())
          .filter((line) => line.startsWith('- '))
          .map((line) => line.slice(2).trim())
          .filter(Boolean);
        return matches;
      };

      const collectReceiptPaths = (receipt) => {
        const entries = [];
        const seen = new Set();
        const add = (value) => {
          const raw = String(value || '').trim();
          if (!raw) return;
          const key = raw;
          if (seen.has(key)) return;
          seen.add(key);
          entries.push(raw);
        };

        const action = receipt?.action || {};
        const metadata = action.metadata || {};

        add(action.path);
        add(action.secondary_path);
        add(metadata.export_src || metadata.import_src || metadata.src);
        add(metadata.export_dst || metadata.import_dst || metadata.dst);
        parseDetailTargets(receipt?.decision?.detail).forEach(add);

        return entries;
      };

      const isApprovalResolutionReceipt = (receipt) =>
        String(receipt?.confinement || '').startsWith('approval-resolution-');

      const shortApprovalId = (receipt) => {
        const detail = String(receipt?.decision?.detail || '');
        const match = detail.match(/approval_id:\s*([a-f0-9-]+)/i);
        if (!match) return '';
        const id = match[1];
        return id.length > 8 ? id.slice(-8) : id;
      };

      const approvalSummaryText = (receipt) => {
        const rawDetail = String(receipt?.decision?.detail || '');
        const bulkMatch = rawDetail.match(/^bulk approval \((\d+)\)\s+(approved|denied|resolved)/i);
        const verdict = String(receipt?.decision?.verdict || '').toLowerCase();
        const status = verdict.includes('deny')
          ? 'denied'
          : verdict.includes('allow')
            ? 'approved'
            : 'resolved';
        if (bulkMatch) {
          const count = bulkMatch[1];
          const targets = collectReceiptPaths(receipt).map(aliasRuntimePath);
          const preview = targets.slice(0, 3).join(', ');
          const suffix = targets.length > 3 ? ' + more (view more in details)' : '';
          return `bulk approval (${count}) ${status} by user${targets.length ? `\ntargets: ${preview}${suffix}` : ''}`;
        }
        const idShort = shortApprovalId(receipt);
        const headline = idShort
          ? `approval \`${idShort}\` ${status} by user`
          : `approval ${status} by user`;

        const targets = collectReceiptPaths(receipt).map(aliasRuntimePath);
        if (!targets.length) return headline;

        const preview = targets.slice(0, 3).join(', ');
        const suffix = targets.length > 3 ? ' + more (view more in details)' : '';
        return `${headline}\ntargets: ${preview}${suffix}`;
      };

      const detailsText = (receipt) => {
        const aliasedDetail = aliasRuntimeText(receipt?.decision?.detail || 'No detail provided');
        if (!isApprovalResolutionReceipt(receipt)) return aliasedDetail;
        if (aliasedDetail.includes('\ntargets:')) return aliasedDetail;

        const targets = collectReceiptPaths(receipt).map(aliasRuntimePath);
        if (!targets.length) return aliasedDetail;
        return `${aliasedDetail}\n${targets.length ? `\ntargets:\n${targets.map((target) => `- ${target}`).join('\n')}` : ''}`;
      };
      
      container.innerHTML = result.items.map(r => {
        // Get command summary from metadata when debug details are not shown
        const commandSummary = r.action?.metadata?._command_summary || '';
        const summaryText = isApprovalResolutionReceipt(r)
          ? approvalSummaryText(r)
          : aliasRuntimeText(r.decision.detail || 'No detail provided');
        
        return `
        <div class="timeline-item">
          <div class="timeline-marker ${verdictClass(r.decision.verdict)}">${verdictIcon(r.decision.verdict)}</div>
          <div class="timeline-content">
            <div class="timeline-header">
              <span class="timeline-title">${esc(r.action.operation)}</span>
              <span class="timeline-time">${formatTimelineTimestamp(r.timestamp, filters.date)}</span>
            </div>
            <div class="timeline-body">
              <span class="chip chip-${verdictClass(r.decision.verdict)}">${esc(r.decision.verdict)}</span>
              <span class="chip">${esc(r.decision.reason)}</span>
              ${r.zone ? `<span class="chip">${esc(r.zone)}</span>` : ''}
            </div>
            <div class="timeline-details">
              <div class="text-muted" style="font-size: 0.875rem; white-space: pre-line;">${esc(summaryText)}</div>
              
              ${!showDetails && commandSummary ? `
                <div class="timeline-summary">
                  <div class="form-hint">Command: <span class="mono">${esc(aliasRuntimeText(commandSummary))}</span></div>
                </div>
              ` : ''}
              
              ${showDetails ? `
                <div class="timeline-operator-details">
                  <div class="mt-2 text-muted" style="font-size: 0.875rem; white-space: pre-line;">${esc(detailsText(r))}</div>
                  ${r.diff_summary ? `<div class="mt-2">${formatDiff(r.diff_summary)}</div>` : '<div class="mt-2 form-hint">Diff summary: not available</div>'}
                  <div class="mt-2 form-hint">Command: <span class="mono">${esc(aliasRuntimeText(r.action.process?.command || '-'))}</span></div>
                </div>
              ` : ''}
            </div>
          </div>
        </div>
        `;
      }).join('');
      
      renderReceiptPagination();
      
    } catch (err) {
      container.innerHTML = `
        <div class="alert alert-error">
          <span class="alert-icon">✕</span>
          <div class="alert-content">
            <div class="alert-title">Failed to Load Receipts</div>
            <div class="alert-message">${esc(err.message)}</div>
          </div>
        </div>
      `;
    }
  }

  async function loadUiLogs() {
    const container = document.getElementById('receipts-list');
    if (!container) return;

    const { limit, offset, filters } = state.logs;
    const params = new URLSearchParams();
    params.set('limit', limit);
    params.set('offset', offset);
    if (filters.level) params.set('level', filters.level);
    if (filters.source) params.set('source', filters.source);
    if (filters.q) params.set('q', filters.q);

    try {
      container.innerHTML = '<div class="loading"><div class="spinner"></div></div>';

      const result = await api(`/api/ui/logs?${params}`);
      state.logs.items = result.items;
      state.logs.total = result.total;
      state.logs.hasMore = result.has_more;

      if (!result.items.length) {
        container.innerHTML = `
          <div class="empty-state">
            <div class="empty-state-icon">🪵</div>
            <div class="empty-state-title">No Logs</div>
            <div class="empty-state-description">No control-panel log entries match your current filters.</div>
          </div>
        `;
        return;
      }

      const levelClass = (level) => {
        const normalized = String(level || '').toLowerCase();
        if (normalized === 'error') return 'deny';
        if (normalized === 'warning') return 'require_approval';
        return 'allow';
      };

      const levelIcon = (level) => {
        const normalized = String(level || '').toLowerCase();
        if (normalized === 'error') return '✕';
        if (normalized === 'warning') return '!';
        return 'ℹ';
      };

      container.innerHTML = result.items.map((item) => {
        const normalizedLevel = String(item.level || 'info').toLowerCase();
        const source = String(item.source || 'ui');
        const details = item.details ? JSON.stringify(item.details, null, 2) : '';
        return `
          <div class="timeline-item">
            <div class="timeline-marker ${levelClass(normalizedLevel)}">${levelIcon(normalizedLevel)}</div>
            <div class="timeline-content">
              <div class="timeline-header">
                <span class="timeline-title">${esc(item.message || '')}</span>
                <span class="timeline-time">${formatTimelineTimestamp(item.timestamp, '')}</span>
              </div>
              <div class="timeline-body">
                <span class="chip chip-${levelClass(normalizedLevel)}">${esc(normalizedLevel)}</span>
                <span class="chip">${esc(source)}</span>
              </div>
              ${details ? `
                <div class="timeline-details">
                  <div class="form-hint">Details</div>
                  <pre class="mono timeline-log-details">${esc(details)}</pre>
                </div>
              ` : ''}
            </div>
          </div>
        `;
      }).join('');

      renderLogPagination();
    } catch (err) {
      container.innerHTML = `
        <div class="alert alert-error">
          <span class="alert-icon">✕</span>
          <div class="alert-content">
            <div class="alert-title">Failed to Load Logs</div>
            <div class="alert-message">${esc(err.message)}</div>
          </div>
        </div>
      `;
    }
  }

  function renderReceiptPagination() {
    const container = document.getElementById('receipts-pagination');
    if (!container) return;
    
    const { total, limit, offset, hasMore } = state.receipts;
    const totalPages = Math.ceil(total / limit);
    const currentPage = Math.floor(offset / limit) + 1;
    
    if (totalPages <= 1) {
      container.innerHTML = `<span class="text-muted">${total} record(s)</span>`;
      return;
    }
    
    let html = `
      <button class="pagination-btn" ${offset === 0 ? 'disabled' : ''} data-offset="0">First</button>
      <button class="pagination-btn" ${offset === 0 ? 'disabled' : ''} data-offset="${Math.max(0, offset - limit)}">Prev</button>
      <span class="text-muted">Page ${currentPage} of ${totalPages}</span>
      <button class="pagination-btn" ${!hasMore ? 'disabled' : ''} data-offset="${offset + limit}">Next</button>
      <button class="pagination-btn" ${!hasMore ? 'disabled' : ''} data-offset="${(totalPages - 1) * limit}">Last</button>
    `;
    
    container.innerHTML = html;
    
    container.querySelectorAll('.pagination-btn').forEach(btn => {
      btn.addEventListener('click', () => {
        if (btn.disabled) return;
        state.receipts.offset = parseInt(btn.dataset.offset);
        loadReceipts();
      });
    });
  }

  function renderLogPagination() {
    const container = document.getElementById('receipts-pagination');
    if (!container) return;

    const { total, limit, offset, hasMore } = state.logs;
    const totalPages = Math.ceil(total / limit);
    const currentPage = Math.floor(offset / limit) + 1;

    if (totalPages <= 1) {
      container.innerHTML = `<span class="text-muted">${total} record(s)</span>`;
      return;
    }

    const html = `
      <button class="pagination-btn" ${offset === 0 ? 'disabled' : ''} data-offset="0">First</button>
      <button class="pagination-btn" ${offset === 0 ? 'disabled' : ''} data-offset="${Math.max(0, offset - limit)}">Prev</button>
      <span class="text-muted">Page ${currentPage} of ${totalPages}</span>
      <button class="pagination-btn" ${!hasMore ? 'disabled' : ''} data-offset="${offset + limit}">Next</button>
      <button class="pagination-btn" ${!hasMore ? 'disabled' : ''} data-offset="${(totalPages - 1) * limit}">Last</button>
    `;

    container.innerHTML = html;

    container.querySelectorAll('.pagination-btn').forEach((btn) => {
      btn.addEventListener('click', () => {
        if (btn.disabled) return;
        state.logs.offset = parseInt(btn.dataset.offset);
        loadUiLogs();
      });
    });
  }
