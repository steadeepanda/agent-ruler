  // Extracted from assets/ui/10-pages-main.js for page-scoped editing.
  async function renderApprovals(root) {
    root.innerHTML = `
      <div class="card">
        <div class="card-header">
          <div>
            <h3 class="card-title">Pending Approvals</h3>
            <p class="card-description">Review and approve or deny pending operations</p>
          </div>
          <div class="btn-group">
            <button id="approve-all" class="btn btn-success btn-sm">Approve All</button>
            <button id="deny-all" class="btn btn-danger btn-sm">Deny All</button>
          </div>
        </div>
        <div class="card-body">
          <div id="approvals-list"></div>
        </div>
      </div>
    `;
    
    document.getElementById('approve-all').addEventListener('click', approveAll);
    document.getElementById('deny-all').addEventListener('click', denyAll);
    
    await loadApprovals();
  }

  async function loadApprovals() {
    const container = document.getElementById('approvals-list');
    if (!container) return;
    
    try {
      state.approvals = await api('/api/approvals');
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
          <div class="empty-state">
            <div class="empty-state-icon">✓</div>
            <div class="empty-state-title">All Caught Up</div>
            <div class="empty-state-description">No pending approvals at this time.</div>
          </div>
        `;
        return;
      }
      
      container.innerHTML = normalizedApprovals.map(item => `
        <div class="approval-card" data-id="${esc(item.approval.id)}">
          <div class="approval-header">
            <div class="approval-badges">
              <span class="chip chip-warning">Pending</span>
              <span class="chip">${esc(item.approval.reason)}</span>
              <span class="chip">Expires: ${formatRelativeTime(item.approval.expires_at)}</span>
            </div>
          </div>
          <div class="approval-title">${esc(item.why)}</div>
          <div class="approval-meta">
            <div class="approval-meta-item">
              <span class="approval-meta-label">Operation:</span>
              <span class="approval-meta-value">${esc(item.approval.action.operation)}</span>
            </div>
            <div class="approval-meta-item">
              <span class="approval-meta-label">Source:</span>
              <span class="approval-meta-value">${esc(item.resolved_src || '-')}</span>
            </div>
            <div class="approval-meta-item">
              <span class="approval-meta-label">Destination:</span>
              <span class="approval-meta-value">${esc(item.resolved_dst || '-')}</span>
            </div>
            ${item.diff_summary ? `
              <div class="approval-meta-item">
                <span class="approval-meta-label">Changes:</span>
                <span class="approval-meta-value">${formatDiff(item.diff_summary)}</span>
              </div>
            ` : ''}
          </div>
          <div class="approval-actions">
            <a class="btn btn-ghost btn-sm" href="/approvals/${encodeURIComponent(item.approval.id)}">Details</a>
            <button class="btn btn-success btn-sm" data-approve="${esc(item.approval.id)}">Approve</button>
            <button class="btn btn-danger btn-sm" data-deny="${esc(item.approval.id)}">Deny</button>
          </div>
        </div>
      `).join('');
      
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
