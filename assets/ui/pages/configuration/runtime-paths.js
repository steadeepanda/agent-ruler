  // Extracted from assets/ui/20-pages-config.js for configuration-scoped editing.
  // ============================================
  // Runtime Page
  // ============================================

  function renderRuntime(root) {
    const r = state.runtime || {};
    
    root.innerHTML = `
      <div class="grid grid-2">
        <div class="card">
          <div class="card-header">
            <h3 class="card-title">Runtime Paths</h3>
            <span class="chip">Read Mostly</span>
          </div>
          <div class="card-body">
            <div class="table-container">
              <table class="table">
                <tbody>
                  <tr>
                    <td><strong>Ruler Root</strong></td>
                    <td class="mono">${esc(r.ruler_root)}</td>
                  </tr>
                  <tr>
                    <td><strong>Runtime Root</strong></td>
                    <td class="mono">${esc(r.runtime_root)}</td>
                  </tr>
                  <tr>
                    <td><strong>Workspace</strong></td>
                    <td class="mono">${esc(r.workspace)}</td>
                  </tr>
                  <tr>
                    <td><strong>Shared Zone</strong></td>
                    <td class="mono">${esc(r.shared_zone)}</td>
                  </tr>
                  <tr>
                    <td><strong>State Directory</strong></td>
                    <td class="mono">${esc(r.state_dir)}</td>
                  </tr>
                  <tr>
                    <td><strong>Policy File</strong></td>
                    <td class="mono">${esc(r.policy_file)}</td>
                  </tr>
                  <tr>
                    <td><strong>Receipts File</strong></td>
                    <td class="mono">${esc(r.receipts_file)}</td>
                  </tr>
                  <tr>
                    <td><strong>Approvals File</strong></td>
                    <td class="mono">${esc(r.approvals_file)}</td>
                  </tr>
                  <tr>
                    <td><strong>Staged Exports File</strong></td>
                    <td class="mono">${esc(r.staged_exports_file)}</td>
                  </tr>
                  <tr>
                    <td><strong>Default Delivery Dir</strong></td>
                    <td class="mono">${esc(r.default_delivery_dir)}</td>
                  </tr>
                  <tr>
                    <td><strong>Exec Layer Dir</strong></td>
                    <td class="mono">${esc(r.exec_layer_dir)}</td>
                  </tr>
                  <tr>
                    <td><strong>Quarantine Dir</strong></td>
                    <td class="mono">${esc(r.quarantine_dir)}</td>
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
