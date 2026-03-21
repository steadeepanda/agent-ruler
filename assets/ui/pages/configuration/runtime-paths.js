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
      <div class="grid grid-2">
        <div class="card">
          <div class="card-header">
            <h3 class="card-title">Runtime Paths</h3>
            <span class="chip">Read Mostly</span>
          </div>
          <div class="card-body">
            <div class="form-group">
              <label class="form-check">
                <input type="checkbox" id="runtime-hide-paths" class="form-check-input" ${hidePaths ? 'checked' : ''} />
                <span class="form-check-label">Hide paths display</span>
              </label>
              <p class="form-hint">This page defaults to full absolute paths so operators can inspect exact locations.</p>
            </div>
            <div class="table-container">
              <table class="table">
                <tbody>
                  <tr>
                    <td><strong>Ruler Root</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.ruler_root, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Runtime Root</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.runtime_root, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Workspace</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.workspace, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Shared Zone</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.shared_zone, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>State Directory</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.state_dir, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Policy File</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.policy_file, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Receipts File</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.receipts_file, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Approvals File</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.approvals_file, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Staged Exports File</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.staged_exports_file, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Default Delivery Dir</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.default_delivery_dir, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Exec Layer Dir</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.exec_layer_dir, hidePaths))}</td>
                  </tr>
                  <tr>
                    <td><strong>Quarantine Dir</strong></td>
                    <td class="mono">${esc(displayRuntimePath(r.quarantine_dir, hidePaths))}</td>
                  </tr>
                </tbody>
              </table>
            </div>
          </div>
        </div>

        <div class="card">
          <div class="card-header">
            <h3 class="card-title">Editable Paths</h3>
          </div>
          <div class="card-body">
            <div class="form-group">
              <label class="form-label">Shared Zone Path</label>
              <input id="runtime-shared-path" class="form-input" value="${esc(r.shared_zone || '')}" />
              <label class="form-check mt-2">
                <input type="checkbox" id="runtime-shared-absolute" class="form-check-input" checked />
                <span class="form-check-label">Treat as absolute path</span>
              </label>
            </div>

            <div class="form-group">
              <label class="form-label">Default Delivery Directory</label>
              <input id="runtime-delivery-path" class="form-input" value="${esc(r.default_user_destination_dir || r.default_delivery_dir || '')}" />
              <label class="form-check mt-2">
                <input type="checkbox" id="runtime-delivery-absolute" class="form-check-input" checked />
                <span class="form-check-label">Treat as absolute path</span>
              </label>
              <p class="form-hint">Used when Deliver destination is not customized.</p>
            </div>

            <button id="runtime-save-paths" class="btn btn-primary">Save Paths</button>
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
