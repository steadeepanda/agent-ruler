  // Extracted from assets/ui/10-pages-main.js for page-scoped editing.
  async function renderApprovals(root) {
    const runnerOptions = runnerFilterOptions();
    root.innerHTML = `
      <div style="margin-bottom: var(--space-6); display: flex; flex-wrap: wrap; justify-content: space-between; align-items: flex-start; gap: var(--space-4);">
        <div>
          <h2 style="font-size: 1.5rem; font-weight: 600; margin-bottom: var(--space-2); color: var(--text-primary);">Pending Approvals</h2>
          <p style="color: var(--text-muted); font-size: 0.95rem;">Review and manage pending operations</p>
        </div>
        <div class="btn-group" style="align-items: center;">
          <select id="approvals-runner-filter" class="form-select" style="width: auto; height: 100%; min-width: 140px; border-radius: var(--radius);">
            ${runnerOptions.map((runner) => `
              <option value="${esc(runner.id)}" ${state.runnerFilter === runner.id ? 'selected' : ''}>${esc(runner.label)}</option>
            `).join('')}
          </select>
          <button id="approve-all" class="btn btn-success"><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: 4px;"><polyline points="20 6 9 17 4 12"/></svg> Approve All</button>
          <button id="deny-all" class="btn btn-danger"><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: 4px;"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg> Deny All</button>
        </div>
      </div>
      
      <div id="approvals-list" style="display: flex; flex-direction: column; gap: var(--space-4);"></div>
    `;
    
    document.getElementById('approve-all').addEventListener('click', approveAll);
    document.getElementById('deny-all').addEventListener('click', denyAll);
    document.getElementById('approvals-runner-filter').addEventListener('change', (event) => {
      setRunnerFilter(event.target.value);
      loadApprovals();
    });
    
    await loadApprovals();
  }

  async function loadApprovals() {
    const container = document.getElementById('approvals-list');
    if (!container) return;
    
    try {
      const params = new URLSearchParams();
      if (state.runnerFilter && state.runnerFilter !== 'all') {
        params.set('runner', state.runnerFilter);
      }
      const querySuffix = params.toString() ? `?${params}` : '';
      state.approvals = await api(`/api/approvals${querySuffix}`);
      const normalizedApprovals = (Array.isArray(state.approvals) ? state.approvals : [])
        .map((item) => {
          const approval = item?.approval || item || {};
          return {
            approval,
            why: item?.why || '',
            resolved_src: item?.resolved_src || null,
            resolved_dst: item?.resolved_dst || null,
            diff_summary: item?.diff_summary || null
          };
        })
        .filter((item) => !!item.approval?.id);
      
      if (!normalizedApprovals.length) {
        container.innerHTML = `
          <div class="empty-state" style="border: 1px dashed var(--content-border); border-radius: var(--radius-lg); background: var(--content-bg-alt);">
            <div class="empty-state-icon" style="color: var(--success); opacity: 0.8;">✓</div>
            <div class="empty-state-title">All Caught Up</div>
            <div class="empty-state-description">No pending approvals at this time.</div>
          </div>
        `;
        return;
      }
      
      container.innerHTML = `<div class="inbox-list" style="border: 1px solid var(--content-border); border-radius: var(--radius-lg); background: var(--content-bg-alt); overflow: hidden;">
        ${normalizedApprovals.map((item, index) => `
          <div class="inbox-row" style="display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4); border-bottom: ${index === normalizedApprovals.length - 1 ? 'none' : '1px solid var(--content-border)'};">
            <div style="display: flex; justify-content: space-between; align-items: flex-start; gap: var(--space-4); flex-wrap: wrap;">
              <div style="flex: 1 1 300px; min-width: 0;">
                <div style="display: flex; align-items: center; gap: var(--space-2); margin-bottom: 6px;">
                  <span class="chip chip-warning" style="font-size: 0.65rem; padding: 2px 6px; line-height: 1;">Pending</span>
                  <span style="font-weight: 600; font-size: 1rem; color: var(--text-primary);">${esc(item.why)}</span>
                </div>
                <div style="display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2); font-size: 0.85rem; color: var(--text-secondary);">
                  <div style="display: flex; align-items: center; gap: 4px; color: var(--text-primary); font-weight: 500;">
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>
                    <span>${esc(item.approval.action.operation)}</span>
                  </div>
                  <span>•</span>
                  <span>${esc((item.approval.action?.metadata?.runner_id || 'unknown').toString())}</span>
                  <span>•</span>
                  <div class="mono" style="color: var(--info); max-width: 250px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${esc(item.resolved_src || '-')}">${esc(aliasRuntimePath(item.resolved_src || '-'))}</div>
                  ${item.resolved_dst ? `<span>→</span> <div class="mono" style="color: var(--success); max-width: 250px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${esc(item.resolved_dst)}">${esc(aliasRuntimePath(item.resolved_dst))}</div>` : ''}
                </div>
              </div>
              
              <div style="display: flex; flex-direction: column; align-items: flex-end; gap: var(--space-2); flex-shrink: 0;">
                <div style="font-size: 0.8rem; color: var(--text-muted);">
                  Expires ${formatRelativeTime(item.approval.expires_at)}
                </div>
                <div style="display: flex; gap: var(--space-2);">
                  <a class="btn btn-ghost btn-sm" href="/approvals/${encodeURIComponent(item.approval.id)}">Details</a>
                  <button class="btn btn-danger btn-sm" data-deny="${esc(item.approval.id)}">Deny</button>
                  <button class="btn btn-success btn-sm" data-approve="${esc(item.approval.id)}">Approve</button>
                </div>
              </div>
            </div>
            
            ${item.diff_summary ? `
              <div style="font-size: 0.8rem; padding: var(--space-2) var(--space-3); background: var(--content-bg); border-radius: var(--radius-sm); border: 1px dashed var(--content-border); align-self: flex-start;">
                Diff summary: ${formatDiff(item.diff_summary)}
              </div>
            ` : ''}
          </div>
        `).join('')}
      </div>`;
      
      // Bind events
      container.querySelectorAll('[data-approve]').forEach(btn => {
        btn.addEventListener('click', () => approveOne(btn.dataset.approve));
      });
      container.querySelectorAll('[data-deny]').forEach(btn => {
        btn.addEventListener('click', () => denyOne(btn.dataset.deny));
      });
      
    } catch (err) {
      container.innerHTML = `
        <div class="alert alert-error">
          <span class="alert-icon">✕</span>
          <div class="alert-content">
            <div class="alert-title">Failed to Load Approvals</div>
            <div class="alert-message">${esc(err.message)}</div>
          </div>
        </div>
      `;
    }
  }

  async function approveOne(id) {
    try {
      await api(`/api/approvals/${id}/approve`, { method: 'POST' });
      toast('Approved successfully', 'success');
      await Promise.all([refreshStatus(), loadApprovals()]);
    } catch (err) {
      toast(`Failed to approve: ${err.message}`, 'error');
    }
  }

  async function denyOne(id) {
    try {
      await api(`/api/approvals/${id}/deny`, { method: 'POST' });
      toast('Denied successfully', 'success');
      await Promise.all([refreshStatus(), loadApprovals()]);
    } catch (err) {
      toast(`Failed to deny: ${err.message}`, 'error');
    }
  }

  async function approveAll() {
    if (!confirm('Approve all pending requests? This action cannot be undone.')) return;
    try {
      const result = await api('/api/approvals/approve-all', { method: 'POST' });
      toast(`Approved ${result.updated.length} requests`, 'success');
      await Promise.all([refreshStatus(), loadApprovals()]);
    } catch (err) {
      toast(`Failed to approve all: ${err.message}`, 'error');
    }
  }

  async function denyAll() {
    if (!confirm('Deny all pending requests? This action cannot be undone.')) return;
    try {
      const result = await api('/api/approvals/deny-all', { method: 'POST' });
      toast(`Denied ${result.updated.length} requests`, 'success');
      await Promise.all([refreshStatus(), loadApprovals()]);
    } catch (err) {
      toast(`Failed to deny all: ${err.message}`, 'error');
    }
  }

  // ============================================
  // Files Page
  // ============================================
