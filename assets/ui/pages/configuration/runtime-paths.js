  // Extracted from assets/ui/20-pages-config.js for configuration-scoped editing.
  // ============================================
  // Runtime Page
  // ============================================
  const RUNTIME_PATHS_HIDE_STORAGE_KEY = 'ar.runtime.hidePathsDisplay';

  function readRuntimePathsHidePreference() {
    return localStorage.getItem(RUNTIME_PATHS_HIDE_STORAGE_KEY) === '1';
  }

  function displayRuntimePath(rawValue, hidePaths) {
    if (hidePaths) return '[hidden]';
    return String(rawValue || '-');
  }

  function renderRuntime(root) {
    const r = state.runtime || {};
    const hidePaths = readRuntimePathsHidePreference();
    
    root.innerHTML = `
      <div class="settings-container">
        <div class="settings-header">
          <h2 class="settings-title">Runtime Paths</h2>
          <p class="settings-description">Configure execution boundaries and filesystem locations for the Agent Ruler agent.</p>
        </div>

        <!-- Editable Paths Section -->
        <div class="settings-section">
          <div class="settings-section-header">
            <h3>Delivery Paths</h3>
            <p>Customize where files are shared and delivered.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row">
              <label class="form-label">Shared Zone Path</label>
              <div style="display: flex; flex-direction: column; gap: var(--space-2);">
                <input id="runtime-shared-path" class="form-input mono" value="${esc(r.shared_zone || '')}" />
                <label class="form-check" style="margin: 0;">
                  <input type="checkbox" id="runtime-shared-absolute" class="form-check-input" checked />
                  <span class="form-check-label" style="font-size: 0.85rem;">Treat as absolute path</span>
                </label>
              </div>
            </div>
            
            <div class="settings-row mt-4">
              <label class="form-label">Default Delivery Directory</label>
              <div style="display: flex; flex-direction: column; gap: var(--space-2);">
                <input id="runtime-delivery-path" class="form-input mono" value="${esc(r.default_user_destination_dir || r.default_delivery_dir || '')}" />
                <label class="form-check" style="margin: 0;">
                  <input type="checkbox" id="runtime-delivery-absolute" class="form-check-input" checked />
                  <span class="form-check-label" style="font-size: 0.85rem;">Treat as absolute path</span>
                </label>
              </div>
              <p class="form-hint" style="margin-top: var(--space-1);">Used when Deliver destination is not explicitly customized by the policy.</p>
            </div>

            <div class="settings-row mt-4">
              <button id="runtime-save-paths" class="btn btn-primary" style="align-self: flex-start;">Save Changes</button>
            </div>
          </div>
        </div>

        <!-- System Paths Section -->
        <div class="settings-section" style="border-bottom: none;">
          <div class="settings-section-header">
            <h3>System Architecture</h3>
            <p>Read-only paths configured by the environment deployment.</p>
            <div class="mt-4">
              <label class="form-switch">
                <input type="checkbox" id="runtime-hide-paths" class="form-switch-input" ${hidePaths ? 'checked' : ''} />
                <div class="form-switch-text">
                  <span class="form-switch-label" style="font-size: 0.9rem;">Mask true paths</span>
                </div>
              </label>
            </div>
          </div>
          <div class="settings-section-content">
            <div style="border: 1px solid var(--content-border); border-radius: var(--radius-lg); overflow: hidden; background: var(--bg-primary);">
              ${[
                ['Ruler Root', r.ruler_root],
                ['Runtime Root', r.runtime_root],
                ['Workspace', r.workspace],
                ['Shared Zone', r.shared_zone],
                ['State Directory', r.state_dir],
                ['Policy File', r.policy_file],
                ['Receipts File', r.receipts_file],
                ['Approvals File', r.approvals_file],
                ['Staged Exports File', r.staged_exports_file],
                ['Exec Layer Dir', r.exec_layer_dir],
                ['Quarantine Dir', r.quarantine_dir]
              ].map(([label, val], idx, arr) => `
                <div style="display: flex; padding: var(--space-3) var(--space-4); ${idx < arr.length - 1 ? 'border-bottom: 1px solid var(--content-border);' : ''}">
                  <div style="width: 160px; font-size: 0.85rem; font-weight: 500; color: var(--text-secondary);">${esc(label)}</div>
                  <div class="mono" style="flex: 1; font-size: 0.85rem; color: var(--text-primary); word-break: break-all;">${esc(displayRuntimePath(val, hidePaths))}</div>
                </div>
              `).join('')}
            </div>
          </div>
        </div>
      </div>
    `;

    document.getElementById('runtime-save-paths').addEventListener('click', updateRuntimePaths);
    document.getElementById('runtime-hide-paths')?.addEventListener('change', (event) => {
      localStorage.setItem(
        RUNTIME_PATHS_HIDE_STORAGE_KEY,
        event.target.checked ? '1' : '0'
      );
      renderRuntime(root);
    });
  }

  async function updateRuntimePaths() {
    const sharedPath = (document.getElementById('runtime-shared-path')?.value || '').trim();
    const sharedAbs = !!document.getElementById('runtime-shared-absolute')?.checked;
    const deliveryPath = (document.getElementById('runtime-delivery-path')?.value || '').trim();
    const deliveryAbs = !!document.getElementById('runtime-delivery-absolute')?.checked;

    if (!sharedPath || !deliveryPath) {
      toast('Shared zone path and default delivery path are required', 'warning');
      return;
    }

    try {
      await api('/api/runtime/paths', {
        method: 'POST',
        body: {
          shared_zone_path: sharedPath,
          shared_zone_absolute: sharedAbs,
          default_user_destination_path: deliveryPath,
          default_user_destination_absolute: deliveryAbs
        }
      });
      toast('Runtime paths updated', 'success');
      await refreshStatus();
      renderPage();
    } catch (err) {
      toast(`Failed to update runtime paths: ${err.message}`, 'error');
    }
  }
