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
    const claudecodeBridgeMeta = configPayload?.claudecode_bridge || {};
    const claudecodeBridge = claudecodeBridgeMeta.config || {};
    const opencodeBridgeMeta = configPayload?.opencode_bridge || {};
    const opencodeBridge = opencodeBridgeMeta.config || {};
    const appVersion = state.status?.app_version || state.runtime?.app_version || configPayload?.app_version || '0.0.0';
    const configPath = state.runtime?.state_dir ? `${state.runtime.state_dir}/config.yaml` : 'state/config.yaml';
    const bridgeConfigPath = bridgeMeta.config_path || (state.runtime?.runtime_root ? `${state.runtime.runtime_root}/user_data/bridge/openclaw-channel-bridge.generated.json` : 'user_data/bridge/openclaw-channel-bridge.generated.json');
    const claudecodeBridgeConfigPath = claudecodeBridgeMeta.config_path || (state.runtime?.runtime_root ? `${state.runtime.runtime_root}/user_data/bridge/claudecode-telegram-channel-bridge.generated.json` : 'user_data/bridge/claudecode-telegram-channel-bridge.generated.json');
    const opencodeBridgeConfigPath = opencodeBridgeMeta.config_path || (state.runtime?.runtime_root ? `${state.runtime.runtime_root}/user_data/bridge/opencode-telegram-channel-bridge.generated.json` : 'user_data/bridge/opencode-telegram-channel-bridge.generated.json');

    const SETTINGS_TAB_STORAGE_KEY = 'ar.settings.tab';
    const settingsTabIds = ['general', 'openclaw', 'claudecode', 'opencode'];
    const normalizeListValues = (values) => {
      const out = [];
      const seen = new Set();
      (Array.isArray(values) ? values : []).forEach((value) => {
        const trimmed = String(value || '').trim();
        if (!trimmed || seen.has(trimmed)) return;
        seen.add(trimmed);
        out.push(trimmed);
      });
      return out;
    };
    const parseListInput = (raw) => {
      const parts = String(raw || '')
        .split(/,|\r?\n/)
        .map((entry) => entry.trim())
        .filter(Boolean);
      return normalizeListValues(parts);
    };

    const runnerAllowFrom = {
      'claudecode-bridge': normalizeListValues(claudecodeBridge.allow_from || []),
      'opencode-bridge': normalizeListValues(opencodeBridge.allow_from || [])
    };

    const renderAllowFromEditor = (prefix, label) => `
      <div class="settings-row" style="margin-top: var(--space-4);">
        <label class="form-label">${label} Allowed Telegram Sender IDs</label>
        <p class="form-hint">Add one or many sender IDs (comma-separated). Run <code class="mono">/whoami</code> in Telegram to get your sender ID. Use <code class="mono">*</code> only for controlled local testing.</p>
        <div style="display: flex; gap: var(--space-2); align-items: center; margin-bottom: var(--space-3);">
          <input
            id="settings-${prefix}-allow-from-input"
            class="form-input"
            style="max-width: 300px;"
            placeholder="example: 123456789, 987654321 or *"
          />
          <button id="settings-${prefix}-allow-from-add" class="btn btn-secondary btn-sm" type="button">Add IDs</button>
          <button id="settings-${prefix}-allow-from-remove-selected" class="btn btn-danger btn-sm" type="button" disabled>Delete Selected</button>
        </div>
        <div id="settings-${prefix}-allow-from-list" class="domain-list id-list-container" role="list" style="max-height: 200px; overflow-y: auto;"></div>
      </div>
    `;

    // Keep runner bridge blocks visually consistent while allowing runner-local
    // values; this preserves structure without coupling Claude/OpenCode behavior.
    const renderRunnerTelegramSection = (prefix, label, cfg) => `
      <div class="settings-section">
        <div class="settings-section-header">
          <h3>${label} Telegram Bridge</h3>
          <p>Manage the ${label} integration with Telegram. Approvals remain enforced by Agent Ruler.</p>
        </div>
        <div class="settings-section-content">
          <div class="settings-row">
            <label class="form-switch">
              <input type="checkbox" id="settings-${prefix}-enabled" class="form-switch-input" ${cfg.enabled ? 'checked' : ''} />
              <div class="form-switch-text">
                <span class="form-switch-label">Enable Telegram Bridge</span>
                <span class="form-switch-description">Carry both operator notifications and runner conversations across Telegram.</span>
              </div>
            </label>
          </div>
          <div class="settings-row">
            <label class="form-switch">
              <input type="checkbox" id="settings-${prefix}-answer-streaming" class="form-switch-input" ${cfg.answer_streaming_enabled !== false ? 'checked' : ''} />
              <div class="form-switch-text">
                <span class="form-switch-label">Stream answers in Telegram</span>
                <span class="form-switch-description">Progressively update the same reply message while generation is ongoing.</span>
              </div>
            </label>
          </div>
          <div class="settings-row mt-3">
            <label class="form-label">Telegram Bot Token</label>
            <input id="settings-${prefix}-token" type="password" class="form-input" placeholder="Leave empty to keep existing token" autocomplete="off" />
            <p class="form-hint">Current status: ${cfg.bot_token_configured ? 'Configured' : 'Not configured'}</p>
          </div>
          ${renderAllowFromEditor(prefix, label)}
          
          <div class="settings-row-split mt-4">
            <div class="settings-row" style="flex:1;">
              <label class="form-label">Poll Interval (seconds)</label>
              <input id="settings-${prefix}-poll-interval" type="number" min="1" max="300" class="form-input" value="${esc(cfg.poll_interval_seconds || 8)}" />
            </div>
            <div class="settings-row" style="flex:1;">
              <label class="form-label">Decision TTL (seconds)</label>
              <input id="settings-${prefix}-decision-ttl" type="number" min="60" max="604800" class="form-input" value="${esc(cfg.decision_ttl_seconds || 7200)}" />
            </div>
          </div>
          
          <div class="settings-row-split">
            <div class="settings-row" style="flex:1;">
              <label class="form-label">Short ID Length</label>
              <input id="settings-${prefix}-short-id-length" type="number" min="4" max="10" class="form-input" value="${esc(cfg.short_id_length || 6)}" />
            </div>
            <div class="settings-row" style="flex:1;">
              <label class="form-label">State File</label>
              <input id="settings-${prefix}-state-file" class="form-input mono" value="${esc(cfg.state_file || '')}" />
            </div>
          </div>
        </div>
      </div>
    `;

    root.innerHTML = `
      <div class="settings-container">
        <div class="settings-header">
          <h2 class="settings-title">Control Settings</h2>
          <p class="settings-description">Manage core system options, UI behaviors, and Runner bridge integrations.</p>
        </div>
        
        <div class="panel-tabs settings-tabs" role="tablist" style="margin-bottom: var(--space-6); border-bottom: 1px solid var(--content-border);">
          <button type="button" class="panel-tab" data-settings-tab="general" role="tab">General</button>
          <button type="button" class="panel-tab" data-settings-tab="openclaw" role="tab">OpenClaw</button>
          <button type="button" class="panel-tab" data-settings-tab="claudecode" role="tab">Claude Code</button>
          <button type="button" class="panel-tab" data-settings-tab="opencode" role="tab">OpenCode</button>
        </div>

        <section class="settings-tab-panel" data-settings-tab-panel="general">
          <div class="settings-section">
            <div class="settings-section-header">
              <h3>System Version</h3>
              <p>Keep Agent Ruler up to date. Updating preserves your runtime data and config.</p>
            </div>
            <div class="settings-section-content">
              <div class="settings-row" style="background: var(--content-bg); border: 1px solid var(--content-border); padding: var(--space-4); border-radius: var(--radius-lg);">
                <div style="display: flex; align-items: center; gap: var(--space-4);">
                  <div class="chip">v${esc(appVersion)}</div>
                  <button id="settings-check-updates" class="btn btn-secondary btn-sm" type="button">Check for Updates</button>
                  <button id="settings-apply-update" class="btn btn-warning btn-sm" type="button" style="display:none;">Update Now</button>
                </div>
                <p id="settings-update-status" class="form-hint" style="margin-top: var(--space-2); margin-bottom: 0;">Checking release updates…</p>
              </div>
            </div>
          </div>

          <div class="settings-section">
            <div class="settings-section-header">
              <h3>Network & UI</h3>
              <p>Configure where the web interface listens for incoming connections.</p>
            </div>
            <div class="settings-section-content">
              <div class="settings-row">
                <label class="form-label">UI Bind Address</label>
                <input id="settings-ui-bind" class="form-input mono" style="max-width: 300px;" value="${esc(c.ui_bind || state.status?.ui_bind || '127.0.0.1:4622')}" placeholder="127.0.0.1:4622" />
                <p class="form-hint">Applies on the next UI restart.</p>
              </div>
              <div class="settings-row mt-4">
                <label class="form-switch">
                  <input type="checkbox" id="settings-runtime-path-labels" class="form-switch-input" ${state.pathDisplay?.useRuntimeAliases ? 'checked' : ''} />
                  <div class="form-switch-text">
                    <span class="form-switch-label">Use runtime path labels (Recommended)</span>
                    <span class="form-switch-description">Display only. Collapses paths to aliases like <span class="mono chip" style="padding:0 4px; font-size: 0.75rem;">WORKSPACE_PATH</span>.</span>
                  </div>
                </label>
              </div>
            </div>
          </div>

          <div class="settings-section">
            <div class="settings-section-header">
              <h3>Behavioral Guardrails</h3>
              <p>Tune approval workflows and security fallbacks.</p>
            </div>
            <div class="settings-section-content">
              <div class="settings-row">
                <label class="form-label">Default Approval Wait Timeout</label>
                <div style="display: flex; align-items: center; gap: var(--space-2);">
                  <input id="settings-approval-wait-timeout" type="number" min="1" max="300" class="form-input" style="max-width: 100px; text-align: center;" value="${esc(c.approval_wait_timeout_secs || 90)}" />
                  <span class="form-hint" style="margin:0;">seconds</span>
                </div>
                <p class="form-hint">Safe default is 90s. Agents can override this per decision up to a maximum limit.</p>
              </div>
              <div class="settings-row mt-4" style="border: 1px solid var(--warning-border); border-left: 3px solid var(--warning); padding: var(--space-4); border-radius: var(--radius-lg);">
                <label class="form-switch">
                  <input type="checkbox" id="settings-degraded" class="form-switch-input" ${c.allow_degraded_confinement ? 'checked' : ''} />
                  <div class="form-switch-text">
                    <span class="form-switch-label" style="color: var(--warning);">Allow degraded confinement fallback</span>
                    <span class="form-switch-description">Keep disabled unless your host environment completely blocks namespaces and you explicitly accept weaker isolation.</span>
                  </div>
                </label>
              </div>
            </div>
          </div>
        </section>

        <section class="settings-tab-panel hidden" data-settings-tab-panel="openclaw">
          <div class="settings-section">
            <div class="settings-section-header">
              <h3>Bridge Timers</h3>
              <p>Polling rates and lifetime configurations.</p>
            </div>
            <div class="settings-section-content">
              <div class="settings-row-split">
                <div class="settings-row" style="flex:1;">
                  <label class="form-label">Poll Interval (s)</label>
                  <input id="settings-bridge-poll-interval" type="number" min="1" max="300" class="form-input" value="${esc(b.poll_interval_seconds || 8)}" />
                  <p class="form-hint">How often bridge checks Agent Ruler.</p>
                </div>
                <div class="settings-row" style="flex:1;">
                  <label class="form-label">Decision TTL (s)</label>
                  <input id="settings-bridge-decision-ttl" type="number" min="60" max="604800" class="form-input" value="${esc(b.decision_ttl_seconds || 7200)}" />
                  <p class="form-hint">Mapping validity for inbound replies.</p>
                </div>
              </div>
            </div>
          </div>

          <div class="settings-section">
            <div class="settings-section-header">
              <h3>Bridge Execution</h3>
              <p>Bind targets and state storage.</p>
            </div>
            <div class="settings-section-content">
              <div class="settings-row mb-3">
                <label class="form-label">Short ID Length</label>
                <input id="settings-bridge-short-id-length" type="number" min="4" max="10" class="form-input" value="${esc(b.short_id_length || 6)}" style="max-width: 100px;" />
              </div>
              <div class="settings-row mb-3">
                <label class="form-label">Inbound Bind Address</label>
                <input id="settings-bridge-inbound-bind" class="form-input mono" value="${esc(b.inbound_bind || '127.0.0.1:4661')}" placeholder="127.0.0.1:4661" />
              </div>
              <div class="settings-row">
                <label class="form-label">State File</label>
                <input id="settings-bridge-state-file" class="form-input mono" value="${esc(b.state_file || '')}" />
              </div>
            </div>
          </div>

          <div class="settings-section">
            <div class="settings-section-header">
              <h3>Binaries & Endpoints</h3>
              <p>External path requirements and derived URLs.</p>
            </div>
            <div class="settings-section-content">
              <div class="settings-row mb-3">
                <label class="form-label">OpenClaw CLI Binary</label>
                <input id="settings-bridge-openclaw-bin" class="form-input mono" value="${esc(b.openclaw_bin || 'openclaw')}" />
              </div>
              <div class="settings-row mb-3">
                <label class="form-label">Agent Ruler CLI Binary</label>
                <input id="settings-bridge-agent-ruler-bin" class="form-input mono" value="${esc(b.agent_ruler_bin || 'agent-ruler')}" />
              </div>
              <div class="settings-row" style="background: var(--bg-primary); padding: var(--space-4); border-radius: var(--radius-lg); border: 1px solid var(--content-border);">
                <label class="form-label" style="margin-bottom: var(--space-2);">Derived URLs</label>
                <div class="mono" style="font-size: 0.85rem; margin-bottom: 6px; color: var(--text-secondary);">ruler_url: <span style="color: var(--text-primary);">${esc(b.ruler_url || '')}</span></div>
                <div class="mono" style="font-size: 0.85rem; margin-bottom: 6px; color: var(--text-secondary);">public_base_url: <span style="color: var(--text-primary);">${esc(b.public_base_url || '')}</span></div>
                <div class="mono" style="font-size: 0.85rem; color: var(--text-secondary);">runtime_dir: <span style="color: var(--text-primary);">${esc(b.runtime_dir || '')}</span></div>
              </div>
            </div>
          </div>
        </section>

        <section class="settings-tab-panel hidden" data-settings-tab-panel="claudecode">
          ${renderRunnerTelegramSection('claudecode-bridge', 'Claude Code', claudecodeBridge)}
        </section>

        <section class="settings-tab-panel hidden" data-settings-tab-panel="opencode">
          ${renderRunnerTelegramSection('opencode-bridge', 'OpenCode', opencodeBridge)}
        </section>

        <div class="settings-section" style="border-bottom: none; padding-bottom: 0;">
          <div class="settings-section-header">
             <h3>Save Changes</h3>
             <p>Review changes before applying them to the configuration profile.</p>
          </div>
          <div class="settings-section-content">
             <button id="settings-save-structured" class="btn btn-primary" style="align-self: flex-start;">Save Control Settings</button>
          </div>
        </div>

        <div class="settings-section" style="border-top: 1px dashed var(--content-border); margin-top: var(--space-8);">
          <div class="settings-section-header">
            <h3>State Locations</h3>
            <p>Paths to the actual stored YAML/JSON configuration files.</p>
          </div>
          <div class="settings-section-content">
            <div style="display: flex; flex-direction: column; gap: var(--space-3); background: var(--bg-primary); padding: var(--space-4); border-radius: var(--radius-lg); border: 1px solid var(--content-border);">
              <div class="mono" style="font-size: 0.85rem; word-break: break-all; color: var(--text-secondary);">${esc(aliasRuntimePath(configPath))}</div>
              <div class="mono" style="font-size: 0.85rem; word-break: break-all; color: var(--text-secondary);">${esc(aliasRuntimePath(bridgeConfigPath))}</div>
              <div class="mono" style="font-size: 0.85rem; word-break: break-all; color: var(--text-secondary);">${esc(aliasRuntimePath(claudecodeBridgeConfigPath))}</div>
              <div class="mono" style="font-size: 0.85rem; word-break: break-all; color: var(--text-secondary);">${esc(aliasRuntimePath(opencodeBridgeConfigPath))}</div>
            </div>
          </div>
        </div>
      </div>
    `;

    const syncAllowFromDeleteState = (prefix) => {
      const removeSelectedBtn = document.getElementById(`settings-${prefix}-allow-from-remove-selected`);
      const listEl = document.getElementById(`settings-${prefix}-allow-from-list`);
      if (!removeSelectedBtn || !listEl) return;
      const selectedCount = listEl.querySelectorAll('.bridge-allow-from-select:checked').length;
      removeSelectedBtn.disabled = selectedCount === 0;
    };

    const renderAllowFromList = (prefix) => {
      const listEl = document.getElementById(`settings-${prefix}-allow-from-list`);
      if (!listEl) return;
      const rows = runnerAllowFrom[prefix] || [];
      if (!rows.length) {
        listEl.innerHTML = '<div class="text-muted">No allowed sender IDs configured yet.</div>';
        syncAllowFromDeleteState(prefix);
        return;
      }
      listEl.innerHTML = rows.map((entry, index) => `
        <div class="domain-item id-list-item" data-index="${index}">
          <label class="form-check">
            <input type="checkbox" class="form-check-input bridge-allow-from-select" data-prefix="${prefix}" data-index="${index}" />
            <span class="form-check-label mono">${esc(entry)}</span>
          </label>
          <div class="domain-item-actions">
            <button class="btn btn-ghost btn-sm bridge-allow-from-remove" data-prefix="${prefix}" data-index="${index}" type="button" aria-label="Remove ${esc(entry)}">✕</button>
          </div>
        </div>
      `).join('');
      syncAllowFromDeleteState(prefix);
    };

    const wireAllowFromEditor = (prefix, label) => {
      const addInput = document.getElementById(`settings-${prefix}-allow-from-input`);
      const addButton = document.getElementById(`settings-${prefix}-allow-from-add`);
      const removeSelectedBtn = document.getElementById(`settings-${prefix}-allow-from-remove-selected`);
      const listEl = document.getElementById(`settings-${prefix}-allow-from-list`);
      if (!addInput || !addButton || !removeSelectedBtn || !listEl) return;

      const addValues = () => {
        const values = parseListInput(addInput.value);
        if (!values.length) {
          toast(`Enter one or more ${label} sender IDs`, 'warning');
          return;
        }
        runnerAllowFrom[prefix] = normalizeListValues([
          ...(runnerAllowFrom[prefix] || []),
          ...values
        ]);
        addInput.value = '';
        renderAllowFromList(prefix);
      };

      addButton.addEventListener('click', addValues);
      addInput.addEventListener('keydown', (event) => {
        if (event.key !== 'Enter') return;
        event.preventDefault();
        addValues();
      });

      removeSelectedBtn.addEventListener('click', () => {
        const selected = Array.from(
          listEl.querySelectorAll('.bridge-allow-from-select:checked')
        )
          .map((node) => Number(node.dataset.index))
          .filter((index) => Number.isInteger(index))
          .sort((a, b) => b - a);
        if (!selected.length) return;
        selected.forEach((index) => {
          runnerAllowFrom[prefix].splice(index, 1);
        });
        renderAllowFromList(prefix);
      });

      listEl.addEventListener('change', (event) => {
        if (!(event.target instanceof HTMLElement)) return;
        if (event.target.classList.contains('bridge-allow-from-select')) {
          syncAllowFromDeleteState(prefix);
        }
      });

      listEl.addEventListener('click', (event) => {
        const target = event.target;
        if (!(target instanceof HTMLElement)) return;
        const removeBtn = target.closest('.bridge-allow-from-remove');
        if (!removeBtn) return;
        const index = Number(removeBtn.dataset.index);
        if (!Number.isInteger(index)) return;
        runnerAllowFrom[prefix].splice(index, 1);
        renderAllowFromList(prefix);
      });

      renderAllowFromList(prefix);
    };

    const activateSettingsTab = (tabId) => {
      const resolvedTab = settingsTabIds.includes(tabId) ? tabId : 'general';
      const tabButtons = Array.from(document.querySelectorAll('[data-settings-tab]'));
      const tabPanels = Array.from(document.querySelectorAll('[data-settings-tab-panel]'));
      tabButtons.forEach((button) => {
        const isActive = button.getAttribute('data-settings-tab') === resolvedTab;
        button.classList.toggle('active', isActive);
        button.setAttribute('aria-selected', isActive ? 'true' : 'false');
      });
      tabPanels.forEach((panel) => {
        const isActive = panel.getAttribute('data-settings-tab-panel') === resolvedTab;
        panel.classList.toggle('hidden', !isActive);
      });
      localStorage.setItem(SETTINGS_TAB_STORAGE_KEY, resolvedTab);
    };

    Array.from(document.querySelectorAll('[data-settings-tab]')).forEach((button) => {
      button.addEventListener('click', () => {
        const nextTab = button.getAttribute('data-settings-tab') || 'general';
        activateSettingsTab(nextTab);
      });
    });

    const initialTab = localStorage.getItem(SETTINGS_TAB_STORAGE_KEY) || 'general';
    activateSettingsTab(initialTab);
    wireAllowFromEditor('claudecode-bridge', 'Claude Code');
    wireAllowFromEditor('opencode-bridge', 'OpenCode');

    const runtimePathLabelsToggle = document.getElementById('settings-runtime-path-labels');
    if (runtimePathLabelsToggle) {
      runtimePathLabelsToggle.addEventListener('change', (event) => {
        setRuntimeAliasVisibility(!!event.target.checked);
        renderPage();
      });
    }

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
      const buildRunnerBridgePayload = (prefix, label, existingConfig) => {
        const enabled = !!document.getElementById(`settings-${prefix}-enabled`)?.checked;
        const token = (document.getElementById(`settings-${prefix}-token`)?.value || '').trim();
        const answerStreamingEnabled = !!document.getElementById(`settings-${prefix}-answer-streaming`)?.checked;
        const allowFrom = normalizeListValues(runnerAllowFrom[prefix] || []);
        const pollRaw = Number(document.getElementById(`settings-${prefix}-poll-interval`)?.value);
        if (!Number.isFinite(pollRaw) || pollRaw < 1 || pollRaw > 300) {
          throw new Error(`${label} bridge poll interval must be between 1 and 300 seconds`);
        }
        const ttlRaw = Number(document.getElementById(`settings-${prefix}-decision-ttl`)?.value);
        if (!Number.isFinite(ttlRaw) || ttlRaw < 60 || ttlRaw > 604800) {
          throw new Error(`${label} bridge decision TTL must be between 60 and 604800 seconds`);
        }
        const shortIdRaw = Number(document.getElementById(`settings-${prefix}-short-id-length`)?.value);
        if (!Number.isFinite(shortIdRaw) || shortIdRaw < 4 || shortIdRaw > 10) {
          throw new Error(`${label} bridge short ID length must be between 4 and 10`);
        }
        const stateFile = (document.getElementById(`settings-${prefix}-state-file`)?.value || '').trim();
        if (!stateFile) {
          throw new Error(`${label} bridge state file path is required`);
        }
        if (enabled && !token && !existingConfig.bot_token_configured) {
          throw new Error(`${label} bridge requires a bot token when enabled`);
        }

        const payload = {
          enabled,
          answer_streaming_enabled: answerStreamingEnabled,
          poll_interval_seconds: Math.floor(pollRaw),
          decision_ttl_seconds: Math.floor(ttlRaw),
          short_id_length: Math.floor(shortIdRaw),
          state_file: stateFile,
          allow_from: allowFrom
        };
        if (token) {
          payload.bot_token = token;
        }
        return payload;
      };

      try {
        const claudecodeBridgePayload = buildRunnerBridgePayload(
          'claudecode-bridge',
          'Claude Code',
          claudecodeBridge
        );
        const opencodeBridgePayload = buildRunnerBridgePayload(
          'opencode-bridge',
          'OpenCode',
          opencodeBridge
        );
        await api('/api/config/update', {
          method: 'POST',
          body: {
            ui_bind: uiBind,
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
            },
            claudecode_bridge: claudecodeBridgePayload,
            opencode_bridge: opencodeBridgePayload
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
