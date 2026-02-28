  // Extracted from assets/ui/20-pages-config.js for configuration-scoped editing.
  // ============================================
  // Settings Page
  // ============================================

  async function renderSettings(root) {
    let configPayload = null;
    try {
      configPayload = await api('/api/config');
      state.config = configPayload.config || null;
    } catch (err) {
      toast(`Failed to load control settings: ${err.message}`, 'error');
    }

    const c = state.config || {};
    const bridgeMeta = configPayload?.openclaw_bridge || {};
    const b = bridgeMeta.config || {};
    const appVersion = state.status?.app_version || state.runtime?.app_version || configPayload?.app_version || '0.0.0';
    const configPath = state.runtime?.state_dir ? `${state.runtime.state_dir}/config.yaml` : 'state/config.yaml';
    const bridgeConfigPath = bridgeMeta.config_path || (state.runtime?.runtime_root ? `${state.runtime.runtime_root}/user_data/bridge/openclaw-channel-bridge.generated.json` : 'user_data/bridge/openclaw-channel-bridge.generated.json');

    root.innerHTML = `
      <div class="card">
        <div class="card-header">
          <h3 class="card-title">Control Panel Settings</h3>
        </div>
        <div class="card-body">
          <div class="form-group">
            <label class="form-label">Ruler Version</label>
            <div class="btn-group">
              <div class="chip">v${esc(appVersion)}</div>
              <button id="settings-check-updates" class="btn btn-ghost btn-sm btn-chip-match" type="button">Check for Updates</button>
              <button id="settings-apply-update" class="btn btn-warning btn-sm" type="button" style="display:none;">Update Now</button>
            </div>
            <p id="settings-update-status" class="form-hint mt-2">Checking release updates…</p>
          </div>
          <div class="form-group">
            <label class="form-label">UI Bind Address</label>
            <input id="settings-ui-bind" class="form-input" value="${esc(c.ui_bind || state.status?.ui_bind || '127.0.0.1:4622')}" placeholder="127.0.0.1:4622" />
            <p class="form-hint">Applies on next UI restart.</p>
          </div>
          <div class="form-group">
            <label class="form-check">
              <input type="checkbox" id="settings-debug-tools" class="form-check-input" ${c.ui_show_debug_tools ? 'checked' : ''} />
              <span class="form-check-label">Show debug tools in UI</span>
            </label>
          </div>
          <div class="form-group">
            <label class="form-check">
              <input type="checkbox" id="settings-degraded" class="form-check-input" ${c.allow_degraded_confinement ? 'checked' : ''} />
              <span class="form-check-label">Allow degraded confinement fallback</span>
            </label>
            <p class="form-hint">Keep disabled unless your host blocks namespaces and you explicitly accept weaker isolation.</p>
          </div>
          <div class="form-group">
            <label class="form-label">Default Approval Wait Timeout (seconds)</label>
            <input
              id="settings-approval-wait-timeout"
              type="number"
              min="1"
              max="300"
              class="form-input"
              value="${esc(c.approval_wait_timeout_secs || 90)}"
            />
            <p class="form-hint">Safe default is 90s. Agents can still override per wait call when needed.</p>
          </div>
          <div class="form-group">
            <label class="form-label">OpenClaw Bridge Poll Interval (seconds)</label>
            <input
              id="settings-bridge-poll-interval"
              type="number"
              min="1"
              max="300"
              class="form-input"
              value="${esc(b.poll_interval_seconds || 8)}"
            />
            <p class="form-hint">How often bridge checks pending approvals on Agent Ruler.</p>
          </div>
          <div class="form-group">
            <label class="form-label">OpenClaw Bridge Decision TTL (seconds)</label>
            <input
              id="settings-bridge-decision-ttl"
              type="number"
              min="60"
              max="604800"
              class="form-input"
              value="${esc(b.decision_ttl_seconds || 7200)}"
            />
            <p class="form-hint">How long short-id decision mappings stay valid for inbound approve/deny replies.</p>
          </div>
          <div class="form-group">
            <label class="form-label">OpenClaw Bridge Short ID Length</label>
            <input
              id="settings-bridge-short-id-length"
              type="number"
              min="4"
              max="10"
              class="form-input"
              value="${esc(b.short_id_length || 6)}"
            />
            <p class="form-hint">Length of operator-facing short IDs used in Telegram/Discord/WhatsApp approval commands.</p>
          </div>
          <div class="form-group">
            <label class="form-label">OpenClaw Bridge Inbound Bind</label>
            <input id="settings-bridge-inbound-bind" class="form-input" value="${esc(b.inbound_bind || '127.0.0.1:4661')}" placeholder="127.0.0.1:4661" />
            <p class="form-hint">OpenClaw approvals hook posts inbound channel events to this address.</p>
          </div>
          <div class="form-group">
            <label class="form-label">OpenClaw Bridge State File</label>
            <input id="settings-bridge-state-file" class="form-input" value="${esc(b.state_file || '')}" />
            <p class="form-hint">Stores seen approvals and short-id mappings for restart-safe behavior.</p>
          </div>
          <div class="form-group">
            <label class="form-label">OpenClaw CLI Binary</label>
            <input id="settings-bridge-openclaw-bin" class="form-input" value="${esc(b.openclaw_bin || 'openclaw')}" />
            <p class="form-hint">CLI used by bridge to send channel messages.</p>
          </div>
          <div class="form-group">
            <label class="form-label">Agent Ruler CLI Binary (Operator Resolution)</label>
            <input id="settings-bridge-agent-ruler-bin" class="form-input" value="${esc(b.agent_ruler_bin || 'agent-ruler')}" />
            <p class="form-hint">Bridge uses this operator CLI for approve/deny button actions.</p>
          </div>
          <div class="form-group">
            <label class="form-label">OpenClaw Bridge URLs</label>
            <div class="mono">ruler_url: ${esc(b.ruler_url || '')}</div>
            <div class="mono">public_base_url: ${esc(b.public_base_url || '')}</div>
            <div class="mono">runtime_dir: ${esc(b.runtime_dir || '')}</div>
            <p class="form-hint">Auto-derived from UI bind and Tailscale availability when settings are loaded/saved.</p>
          </div>
          <button id="settings-save-structured" class="btn btn-primary">Save Control Settings</button>
        </div>
      </div>

      <div class="card mt-5">
        <div class="card-header">
          <h3 class="card-title">Configuration File</h3>
        </div>
        <div class="card-body">
          <div class="mono">${esc(configPath)}</div>
          <div class="mono mt-2">${esc(bridgeConfigPath)}</div>
          <p class="form-hint mt-2">Runtime path edits for shared-zone and default delivery remain available in <a href="/runtime">Runtime Paths</a>.</p>
        </div>
      </div>
    `;

    document.getElementById('settings-save-structured').addEventListener('click', async () => {
      const uiBind = (document.getElementById('settings-ui-bind')?.value || '').trim();
      if (!uiBind) {
        toast('UI bind address is required', 'warning');
        return;
      }
      const waitTimeoutRaw = Number(document.getElementById('settings-approval-wait-timeout')?.value);
      if (!Number.isFinite(waitTimeoutRaw) || waitTimeoutRaw < 1 || waitTimeoutRaw > 300) {
        toast('Approval wait timeout must be between 1 and 300 seconds', 'warning');
        return;
      }
      const waitTimeout = Math.floor(waitTimeoutRaw);
      const bridgePollRaw = Number(document.getElementById('settings-bridge-poll-interval')?.value);
      if (!Number.isFinite(bridgePollRaw) || bridgePollRaw < 1 || bridgePollRaw > 300) {
        toast('OpenClaw bridge poll interval must be between 1 and 300 seconds', 'warning');
        return;
      }
      const bridgeTtlRaw = Number(document.getElementById('settings-bridge-decision-ttl')?.value);
      if (!Number.isFinite(bridgeTtlRaw) || bridgeTtlRaw < 60 || bridgeTtlRaw > 604800) {
        toast('OpenClaw bridge decision TTL must be between 60 and 604800 seconds', 'warning');
        return;
      }
      const bridgeShortIdRaw = Number(document.getElementById('settings-bridge-short-id-length')?.value);
      if (!Number.isFinite(bridgeShortIdRaw) || bridgeShortIdRaw < 4 || bridgeShortIdRaw > 10) {
        toast('OpenClaw bridge short ID length must be between 4 and 10', 'warning');
        return;
      }
      const bridgeInboundBind = (document.getElementById('settings-bridge-inbound-bind')?.value || '').trim();
      if (!bridgeInboundBind) {
        toast('OpenClaw bridge inbound bind is required', 'warning');
        return;
      }
      const bridgeStateFile = (document.getElementById('settings-bridge-state-file')?.value || '').trim();
      if (!bridgeStateFile) {
        toast('OpenClaw bridge state file path is required', 'warning');
        return;
      }
      const bridgeOpenclawBin = (document.getElementById('settings-bridge-openclaw-bin')?.value || '').trim();
      if (!bridgeOpenclawBin) {
        toast('OpenClaw bridge binary is required', 'warning');
        return;
      }
      const bridgeAgentRulerBin = (document.getElementById('settings-bridge-agent-ruler-bin')?.value || '').trim();
      if (!bridgeAgentRulerBin) {
        toast('Agent Ruler bridge binary is required', 'warning');
        return;
      }

      try {
        await api('/api/config/update', {
          method: 'POST',
          body: {
            ui_bind: uiBind,
            ui_show_debug_tools: !!document.getElementById('settings-debug-tools')?.checked,
            allow_degraded_confinement: !!document.getElementById('settings-degraded')?.checked,
            approval_wait_timeout_secs: waitTimeout,
            openclaw_bridge: {
              poll_interval_seconds: Math.floor(bridgePollRaw),
              decision_ttl_seconds: Math.floor(bridgeTtlRaw),
              short_id_length: Math.floor(bridgeShortIdRaw),
              inbound_bind: bridgeInboundBind,
              state_file: bridgeStateFile,
              openclaw_bin: bridgeOpenclawBin,
              agent_ruler_bin: bridgeAgentRulerBin
            }
          }
        });
        toast('Control settings updated', 'success');
        await refreshStatus();
        await renderSettings(root);
      } catch (err) {
        toast(`Failed to update control settings: ${err.message}`, 'error');
      }
    });

    const updateStatusEl = document.getElementById('settings-update-status');
    const checkUpdatesBtn = document.getElementById('settings-check-updates');
    const applyUpdateBtn = document.getElementById('settings-apply-update');

    const renderUpdateStatus = (payload) => {
      if (!updateStatusEl) return;
      if (!payload) {
        updateStatusEl.textContent = 'Update status unavailable.';
        return;
      }
      if (payload.update_available) {
        updateStatusEl.innerHTML = `Update available: <strong>${esc(payload.latest_tag || 'unknown')}</strong> (current v${esc(payload.current_version || appVersion)}).`;
        if (applyUpdateBtn) {
          applyUpdateBtn.style.display = '';
          applyUpdateBtn.dataset.targetTag = String(payload.latest_tag || '').trim();
        }
      } else {
        updateStatusEl.textContent = `Already up to date (v${payload.current_version || appVersion}).`;
        if (applyUpdateBtn) {
          applyUpdateBtn.style.display = 'none';
          applyUpdateBtn.dataset.targetTag = '';
        }
      }
    };

    const checkForUpdates = async (force) => {
      if (updateStatusEl) {
        updateStatusEl.textContent = force ? 'Checking for updates…' : 'Loading update status…';
      }
      const payload = await fetchUpdateStatus({ force: !!force, quiet: !force });
      renderUpdateStatus(payload);
      return payload;
    };

    if (checkUpdatesBtn) {
      checkUpdatesBtn.addEventListener('click', async () => {
        try {
          recordUiEvent('info', 'update-check', 'Manual update check requested');
          await checkForUpdates(true);
        } catch (err) {
          if (updateStatusEl) {
            updateStatusEl.textContent = `Update check failed: ${err.message}`;
          }
          recordUiEvent('warning', 'update-check', `Manual update check failed: ${err.message}`);
        }
      });
    }

    if (applyUpdateBtn) {
      applyUpdateBtn.addEventListener('click', async () => {
        const targetTag = String(applyUpdateBtn.dataset.targetTag || '').trim();
        if (!targetTag) {
          toast('No target update tag available', 'warning');
          recordUiEvent('warning', 'update-apply', 'Update requested without target tag');
          return;
        }
        const confirmed = window.confirm(`Update Agent Ruler to ${targetTag}? Runtime data/config will be preserved.`);
        if (!confirmed) return;

        applyUpdateBtn.disabled = true;
        if (updateStatusEl) {
          updateStatusEl.textContent = `Applying ${targetTag}… this may take a minute.`;
        }
        recordUiEvent('info', 'update-apply', `Applying update to ${targetTag}`);
        try {
          const result = await api('/api/update/apply', {
            method: 'POST',
            body: {
              version: targetTag
            }
          });
          const outcome = result?.result || result;
          const restarted = outcome?.runner_restarted ? 'and restarted managed gateway' : 'but did not restart managed gateway';
          toast(`Updated to ${outcome?.target_tag || targetTag} (${restarted})`, 'success', 6500);
          if (updateStatusEl) {
            updateStatusEl.textContent = `Update applied to ${outcome?.target_tag || targetTag}. Refresh page; restart UI if assets look stale.`;
          }
          recordUiEvent('info', 'update-apply', `Update applied to ${outcome?.target_tag || targetTag}`);
          await refreshStatus();
          await checkForUpdates(true);
        } catch (err) {
          toast(`Update failed: ${err.message}`, 'error', 7000);
          if (updateStatusEl) {
            updateStatusEl.textContent = `Update failed: ${err.message}`;
          }
          recordUiEvent('error', 'update-apply', `Update failed: ${err.message}`);
        } finally {
          applyUpdateBtn.disabled = false;
        }
      });
    }

    checkForUpdates(false).catch((err) => {
      if (updateStatusEl) {
        updateStatusEl.textContent = `Update status unavailable: ${err.message}`;
      }
    });
  }
