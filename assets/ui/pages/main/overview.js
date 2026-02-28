  // Extracted from assets/ui/10-pages-main.js for page-scoped editing.
  // ============================================
  // Overview Page
  // ============================================

  function renderOverview(root) {
    const s = state.status || {};
    const r = state.runtime || {};
    
    root.innerHTML = `
      <div class="grid grid-4 mb-5">
        <div class="card stat-card">
          <div class="stat-label">Pending Approvals</div>
          <div class="stat-value">${s.pending_approvals || 0}</div>
        </div>
        <div class="card stat-card">
          <div class="stat-label">Total Receipts</div>
          <div class="stat-value">${s.receipts_count || 0}</div>
        </div>
        <div class="card stat-card">
          <div class="stat-label">Staged Exports</div>
          <div class="stat-value">${s.staged_count || 0}</div>
        </div>
        <div class="card stat-card">
          <div class="stat-label">Delivered</div>
          <div class="stat-value">${s.delivered_count || 0}</div>
        </div>
      </div>

      <div class="grid grid-2">
        <div class="card">
          <div class="card-header">
            <h3 class="card-title">Runtime Paths</h3>
          </div>
          <div class="card-body">
            <div class="list">
              <div class="list-item">
                <div class="list-item-content">
                  <div class="list-item-title">Workspace</div>
                  <div class="list-item-description mono">${esc(r.workspace)}</div>
                </div>
              </div>
              <div class="list-item">
                <div class="list-item-content">
                  <div class="list-item-title">Shared Zone</div>
                  <div class="list-item-description mono">${esc(r.shared_zone)}</div>
                </div>
              </div>
              <div class="list-item">
                <div class="list-item-content">
                  <div class="list-item-title">User Destination</div>
                  <div class="list-item-description mono">${esc(r.default_user_destination_dir || r.default_delivery_dir)}</div>
                </div>
              </div>
            </div>
          </div>
        </div>

        <div class="card">
          <div class="card-header">
            <h3 class="card-title">Quick Actions</h3>
          </div>
          <div class="card-body">
            <div class="flex flex-col gap-3">
              <a href="/approvals" class="btn btn-primary w-full">
                <span>📋</span> Review Approvals
                ${s.pending_approvals > 0 ? `<span class="chip chip-danger">${s.pending_approvals}</span>` : ''}
              </a>
              <a href="/files" class="btn btn-secondary w-full">
                <span>📁</span> Import / Export Files
              </a>
              <a href="/receipts" class="btn btn-ghost w-full">
                <span>📜</span> View Timeline
              </a>
              <a href="/policy" class="btn btn-ghost w-full">
                <span>⚙️</span> Policy Settings
              </a>
            </div>
          </div>
        </div>
      </div>

      ${s.allow_degraded_confinement ? `
        <div class="alert alert-warning mt-5">
          <span class="alert-icon">⚠️</span>
          <div class="alert-content">
            <div class="alert-title">Degraded Confinement Enabled</div>
            <div class="alert-message">Confinement fallback is enabled. Disable this once your host supports bubblewrap namespaces for full security.</div>
          </div>
        </div>
      ` : ''}
    `;
  }

  // ============================================
  // Approvals Page
  // ============================================
