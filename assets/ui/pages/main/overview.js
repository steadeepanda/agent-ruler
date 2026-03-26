  // Extracted from assets/ui/10-pages-main.js for page-scoped editing.
  // ============================================
  // Overview Page
  // ============================================

  function renderOverview(root) {
    const s = state.status || {};
    const r = state.runtime || {};

    root.innerHTML = `
      <div style="margin-bottom: var(--space-6);">
        <h2 style="font-size: 1.5rem; font-weight: 600; margin-bottom: var(--space-2); color: var(--text-primary);">Dashboard</h2>
        <p style="color: var(--text-muted); font-size: 0.95rem;">Overview of your Agent Ruler workspace and security status.</p>
      </div>

      <div class="grid grid-4" style="margin-bottom: var(--space-6);">
        <div class="card stat-card" style="display: flex; align-items: center; gap: var(--space-4); padding: var(--space-4);">
          <div style="width: 48px; height: 48px; border-radius: var(--radius); background: var(--warning-bg); border: 1px solid var(--warning-border); color: var(--warning); display: flex; align-items: center; justify-content: center; flex-shrink: 0;">
            <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>
          </div>
          <div>
            <div class="stat-value" style="font-size: 1.75rem; line-height: 1; margin: 0;">${s.pending_approvals || 0}</div>
            <div class="stat-label" style="text-transform: none; letter-spacing: normal; font-size: 0.85rem; margin-top: 4px;">Pending Approvals</div>
          </div>
        </div>
        <div class="card stat-card" style="display: flex; align-items: center; gap: var(--space-4); padding: var(--space-4);">
          <div style="width: 48px; height: 48px; border-radius: var(--radius); background: var(--content-bg); border: 1px solid var(--content-border); color: var(--brand-secondary); display: flex; align-items: center; justify-content: center; flex-shrink: 0;">
             <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/></svg>
          </div>
          <div>
            <div class="stat-value" style="font-size: 1.75rem; line-height: 1; margin: 0;">${s.receipts_count || 0}</div>
            <div class="stat-label" style="text-transform: none; letter-spacing: normal; font-size: 0.85rem; margin-top: 4px;">Total Receipts</div>
          </div>
        </div>
        <div class="card stat-card" style="display: flex; align-items: center; gap: var(--space-4); padding: var(--space-4);">
          <div style="width: 48px; height: 48px; border-radius: var(--radius); background: var(--info-bg); border: 1px solid var(--info-border); color: var(--info); display: flex; align-items: center; justify-content: center; flex-shrink: 0;">
            <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="22 3 2 3 10 12.46 10 19 14 21 14 12.46 22 3"/></svg>
          </div>
          <div>
            <div class="stat-value" style="font-size: 1.75rem; line-height: 1; margin: 0;">${s.staged_count || 0}</div>
            <div class="stat-label" style="text-transform: none; letter-spacing: normal; font-size: 0.85rem; margin-top: 4px;">Staged Exports</div>
          </div>
        </div>
        <div class="card stat-card" style="display: flex; align-items: center; gap: var(--space-4); padding: var(--space-4);">
          <div style="width: 48px; height: 48px; border-radius: var(--radius); background: var(--success-bg); border: 1px solid var(--success-border); color: var(--success); display: flex; align-items: center; justify-content: center; flex-shrink: 0;">
             <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
          </div>
          <div>
            <div class="stat-value" style="font-size: 1.75rem; line-height: 1; margin: 0;">${s.delivered_count || 0}</div>
            <div class="stat-label" style="text-transform: none; letter-spacing: normal; font-size: 0.85rem; margin-top: 4px;">Delivered</div>
          </div>
        </div>
      </div>

      <div class="grid" style="grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: var(--space-6);">
        <!-- Quick Actions -->
        <div class="card" style="display: flex; flex-direction: column;">
          <div class="card-header" style="border-bottom: 1px solid var(--content-border); padding: var(--space-4);">
            <h3 class="card-title" style="font-size: 1rem;">Quick Actions</h3>
          </div>
          <div class="card-body" style="flex: 1; display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-4);">
            <a href="/approvals" class="btn ${s.pending_approvals > 0 ? 'btn-primary' : 'btn-ghost'}" style="justify-content: flex-start; width: 100%; border-radius: var(--radius); padding: 0.65rem 1rem;">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: var(--space-2);"><polyline points="20 6 9 17 4 12"/></svg>
              <span style="flex: 1; text-align: left;">Review Approvals</span>
              ${s.pending_approvals > 0 ? `<span class="chip chip-warning" style="margin-left: auto;">${s.pending_approvals}</span>` : ''}
            </a>
            <a href="/files" class="btn btn-ghost" style="justify-content: flex-start; width: 100%; border-radius: var(--radius); padding: 0.65rem 1rem;">
               <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: var(--space-2);"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>
              <span style="flex: 1; text-align: left;">Import / Export Files</span>
            </a>
            <a href="/receipts" class="btn btn-ghost" style="justify-content: flex-start; width: 100%; border-radius: var(--radius); padding: 0.65rem 1rem;">
               <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: var(--space-2);"><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></svg>
              <span style="flex: 1; text-align: left;">View Timeline</span>
            </a>
            <a href="/policy" class="btn btn-ghost" style="justify-content: flex-start; width: 100%; border-radius: var(--radius); padding: 0.65rem 1rem;">
               <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" style="margin-right: var(--space-2);"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>
              <span style="flex: 1; text-align: left;">Policy Settings</span>
            </a>
          </div>
        </div>

        <!-- Runtime Paths -->
        <div class="card" style="display: flex; flex-direction: column;">
          <div class="card-header" style="padding: var(--space-4); border-bottom: 1px solid var(--content-border);">
            <h3 class="card-title" style="font-size: 1rem;">Runtime Boundaries</h3>
          </div>
          <div class="card-body" style="padding: 0;">
            <div style="display: flex; flex-direction: column;">
              <div style="display: flex; align-items: center; gap: var(--space-4); padding: var(--space-4); border-bottom: 1px solid var(--content-border);">
                <div style="width: 32px; height: 32px; border-radius: var(--radius-full); background: var(--info-bg); border: 1px solid var(--info-border); color: var(--info); display: flex; align-items: center; justify-content: center; flex-shrink: 0; font-size: 0.85rem; font-weight: 600;">W</div>
                <div style="flex: 1; min-width: 0;">
                  <div style="font-size: 0.85rem; font-weight: 500; color: var(--text-primary); margin-bottom: 2px;">Workspace</div>
                  <div class="mono" style="font-size: 0.8rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${esc(aliasRuntimePath(r.workspace))}">${esc(aliasRuntimePath(r.workspace))}</div>
                </div>
              </div>
              <div style="display: flex; align-items: center; gap: var(--space-4); padding: var(--space-4); border-bottom: 1px solid var(--content-border);">
                <div style="width: 32px; height: 32px; border-radius: var(--radius-full); background: var(--warning-bg); border: 1px solid var(--warning-border); color: var(--warning); display: flex; align-items: center; justify-content: center; flex-shrink: 0; font-size: 0.85rem; font-weight: 600;">S</div>
                <div style="flex: 1; min-width: 0;">
                  <div style="font-size: 0.85rem; font-weight: 500; color: var(--text-primary); margin-bottom: 2px;">Shared Zone</div>
                  <div class="mono" style="font-size: 0.8rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${esc(aliasRuntimePath(r.shared_zone))}">${esc(aliasRuntimePath(r.shared_zone))}</div>
                </div>
              </div>
              <div style="display: flex; align-items: center; gap: var(--space-4); padding: var(--space-4);">
                <div style="width: 32px; height: 32px; border-radius: var(--radius-full); background: var(--success-bg); border: 1px solid var(--success-border); color: var(--success); display: flex; align-items: center; justify-content: center; flex-shrink: 0; font-size: 0.85rem; font-weight: 600;">D</div>
                <div style="flex: 1; min-width: 0;">
                  <div style="font-size: 0.85rem; font-weight: 500; color: var(--text-primary); margin-bottom: 2px;">User Destination</div>
                  <div class="mono" style="font-size: 0.8rem; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;" title="${esc(aliasRuntimePath(r.default_user_destination_dir || r.default_delivery_dir))}">${esc(aliasRuntimePath(r.default_user_destination_dir || r.default_delivery_dir))}</div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      ${s.allow_degraded_confinement ? `
        <div class="alert alert-warning" style="margin-top: var(--space-6); display: flex; gap: var(--space-3); padding: var(--space-4); background: var(--warning-bg); border: 1px solid var(--warning-border); border-radius: var(--radius-lg);">
          <div style="color: var(--warning); display: flex; align-items: flex-start; margin-top: 2px;"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg></div>
          <div>
            <div style="font-weight: 600; color: var(--text-primary); margin-bottom: 4px;">Degraded Confinement Enabled</div>
            <div style="font-size: 0.85rem; color: var(--warning);">Confinement fallback is enabled. Disable this once your host supports bubblewrap namespaces for full security.</div>
          </div>
        </div>
      ` : ''}
    `;
  }

  // ============================================
  // Approvals Page
  // ============================================
