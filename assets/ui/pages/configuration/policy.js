  // Extracted from assets/ui/20-pages-config.js for configuration-scoped editing.
  // ============================================
  // Policy Page
  // ============================================

  async function renderPolicy(root) {
    try {
      state.policy = await api('/api/policy');
      state.profiles = await api('/api/policy/profiles');
      state.domainPresets = await api('/api/policy/domain-presets');
    } catch (err) {
      toast(`Failed to load policy: ${err.message}`, 'error');
    }
    
    const p = state.policy || {};
    const profiles = state.profiles || [];
    const rules = p.rules || {};
    const fsRules = rules.filesystem || {};
    const netRules = rules.network || {};
    const elevationRules = rules.elevation || {};
    const execRules = rules.execution || {};
    const persistRules = rules.persistence || {};
    const safeguards = p.safeguards || {};
    const normalizeDomainHost = (value) => {
      const raw = String(value || '').trim().toLowerCase();
      if (!raw) return '';
      const withoutScheme = raw.replace(/^https?:\/\//, '');
      const host = withoutScheme.split('/')[0].split(':')[0].trim();
      if (!host) return '';
      if (!/^[a-z0-9.-]+$/.test(host)) return '';
      return host;
    };

    const effectiveProfileId = p.profile === 'user_custom' ? 'custom' : p.profile;
    const activeProfile = profiles.find(pr => pr.id === effectiveProfileId) || null;
    const canCustomizeRules = !!activeProfile?.allow_rule_customization;
    const canCustomizeElevation = !!activeProfile?.allow_elevation_customization;
    const canCustomizeNetwork = activeProfile ? !!activeProfile.allow_network_customization : true;
    const canCustomizeDomains = activeProfile ? !!activeProfile.allow_domain_customization : true;
    const canCreateCustomProfile = !!activeProfile?.can_create_custom_profile;
    const customProfileActive = effectiveProfileId === 'custom';
    const removePresetDomains = customProfileActive;
    
    // Allowlist (POST) domains
    const safeDomainPresets = Array.isArray(state.domainPresets?.post_allowlist_defaults)
      ? state.domainPresets.post_allowlist_defaults
      : (Array.isArray(state.domainPresets?.safe_defaults) ? state.domainPresets.safe_defaults : []);
    const allowlistedHosts = Array.isArray(netRules.allowlist_hosts) ? netRules.allowlist_hosts : [];
    const allowlistRows = [];
    const allowlistIndex = new Map();
    const addAllowlistRow = (host, preset, enabled) => {
      const normalizedHost = normalizeDomainHost(host);
      if (!normalizedHost) return;
      const existing = allowlistIndex.get(normalizedHost);
      if (existing) {
        existing.preset = existing.preset || preset;
        existing.enabled = existing.enabled || enabled;
        return;
      }
      const entry = { host: normalizedHost, preset: !!preset, enabled: !!enabled };
      allowlistRows.push(entry);
      allowlistIndex.set(normalizedHost, entry);
    };
    if (!customProfileActive) {
      safeDomainPresets.forEach(host => addAllowlistRow(host, true, allowlistedHosts.includes(host)));
    }
    allowlistedHosts.forEach(host => addAllowlistRow(host, false, true));

    // GET domain list presets
    const denylistPresets = Array.isArray(state.domainPresets?.get_allowlist_defaults)
      ? state.domainPresets.get_allowlist_defaults
      : (Array.isArray(state.domainPresets?.denylist_defaults) ? state.domainPresets.denylist_defaults : []);
    const denylistedHosts = Array.isArray(netRules.denylist_hosts) ? netRules.denylist_hosts : [];
    const denylistRows = [];
    const denylistIndex = new Map();
    const addDenylistRow = (host, preset, enabled) => {
      const normalizedHost = normalizeDomainHost(host);
      if (!normalizedHost) return;
      const existing = denylistIndex.get(normalizedHost);
      if (existing) {
        existing.preset = existing.preset || preset;
        existing.enabled = existing.enabled || enabled;
        return;
      }
      const entry = { host: normalizedHost, preset: !!preset, enabled: !!enabled };
      denylistRows.push(entry);
      denylistIndex.set(normalizedHost, entry);
    };
    if (!customProfileActive) {
      denylistPresets.forEach(host => addDenylistRow(host, true, denylistedHosts.includes(host)));
    }
    denylistedHosts.forEach(host => addDenylistRow(host, false, true));

    const elevationUseAllowlist = elevationRules.use_allowlist !== false;
    const managerOrder = ['apt', 'pip', 'npm', 'cargo', 'custom'];
    const normalizePackageName = (value) => {
      const normalized = String(value || '').trim().toLowerCase();
      if (!normalized) return '';
      if (!/^[a-z0-9@/._+-]+$/.test(normalized)) return '';
      return normalized;
    };
    const normalizeManager = (value) => {
      const manager = String(value || '').trim().toLowerCase();
      return managerOrder.includes(manager) ? manager : 'custom';
    };
    const allowPackagePresets = state.domainPresets?.allowlisted_packages || {};
    const denyPackagePresets = state.domainPresets?.denylisted_packages || {};
    const activeAllowedPackages = Array.isArray(elevationRules.allowed_packages) ? elevationRules.allowed_packages : [];
    const activeDeniedPackages = Array.isArray(elevationRules.denied_packages) ? elevationRules.denied_packages : [];
    const normalizePackageList = (items) => {
      const normalized = [];
      const seen = new Set();
      items.forEach((item) => {
        const pkg = normalizePackageName(item);
        if (!pkg || seen.has(pkg)) return;
        seen.add(pkg);
        normalized.push(pkg);
      });
      return normalized;
    };
    const inferManager = (name, presets) => {
      for (const manager of managerOrder) {
        if (manager === 'custom') continue;
        const list = Array.isArray(presets?.[manager]) ? presets[manager] : [];
        if (list.map(normalizePackageName).includes(name)) {
          return manager;
        }
      }
      return 'custom';
    };
    const buildPackageRows = (presets, activeList) => {
      const activeSet = new Set(normalizePackageList(activeList));
      const rows = [];
      const index = new Map();
      const addRow = (manager, pkg, preset, enabled) => {
        const normalizedPkg = normalizePackageName(pkg);
        if (!normalizedPkg) return;
        const normalizedManager = normalizeManager(manager);
        const key = `${normalizedManager}:${normalizedPkg}`;
        const existing = index.get(key);
        if (existing) {
          existing.preset = existing.preset || preset;
          existing.enabled = existing.enabled || enabled;
          return;
        }
        const entry = {
          manager: normalizedManager,
          packageName: normalizedPkg,
          preset: !!preset,
          enabled: !!enabled
        };
        rows.push(entry);
        index.set(key, entry);
      };

      for (const manager of managerOrder) {
        if (manager === 'custom') continue;
        const list = Array.isArray(presets?.[manager]) ? presets[manager] : [];
        list.forEach((pkg) => {
          const normalizedPkg = normalizePackageName(pkg);
          if (!normalizedPkg) return;
          addRow(manager, normalizedPkg, true, activeSet.has(normalizedPkg));
        });
      }

      activeSet.forEach((pkg) => {
        const normalizedPkg = normalizePackageName(pkg);
        if (!normalizedPkg) return;
        const manager = inferManager(normalizedPkg, presets);
        addRow(manager, normalizedPkg, false, true);
      });

      rows.sort((a, b) => {
        const managerDelta = managerOrder.indexOf(a.manager) - managerOrder.indexOf(b.manager);
        if (managerDelta !== 0) return managerDelta;
        return a.packageName.localeCompare(b.packageName);
      });

      return rows;
    };
    const allowPackageRows = buildPackageRows(allowPackagePresets, activeAllowedPackages);
    const denyPackageRows = buildPackageRows(denyPackagePresets, activeDeniedPackages);
    const toSafePositiveInt = (value, fallback = 40) => {
      const n = Number(value);
      if (!Number.isFinite(n) || n < 1) return fallback;
      return Math.min(Math.floor(n), Number.MAX_SAFE_INTEGER);
    };
    const massDeleteThreshold = toSafePositiveInt(safeguards.mass_delete_threshold, 40);

    const dispLabel = (value) => {
      if (value === 'allow') return 'Allow';
      if (value === 'approval') return 'Approval';
      if (value === 'deny') return 'Deny';
      return value || '-';
    };

    // Helper to render domain list items
    const renderDomainListItems = (containerId, rows, listType, canEdit) => {
      const container = document.getElementById(containerId);
      if (!container) return;
      if (!rows.length) {
        container.innerHTML = '<div class="text-muted">No domains configured yet.</div>';
        return;
      }
      container.innerHTML = rows.map((entry, index) => `
        <div class="domain-item" data-index="${index}">
          <label class="form-check">
            <input type="checkbox" class="form-check-input domain-enabled" data-list="${listType}" data-index="${index}" ${entry.enabled ? 'checked' : ''} ${canEdit ? '' : 'disabled'} />
            <span class="form-check-label mono">${esc(entry.host)}</span>
          </label>
          <div class="domain-item-actions">
            ${entry.preset ? '<span class="chip">safe default</span>' : '<span class="chip">custom</span>'}
            ${(entry.preset && !removePresetDomains) ? '' : `<button class="btn btn-ghost btn-sm domain-remove" data-list="${listType}" data-index="${index}" type="button" aria-label="Remove ${esc(entry.host)}" ${canEdit ? '' : 'disabled'}>✕</button>`}
          </div>
        </div>
      `).join('');
    };

    const renderPackageListItems = (containerId, rows, listType, canEdit) => {
      const container = document.getElementById(containerId);
      if (!container) return;
      if (!rows.length) {
        container.innerHTML = '<div class="text-muted">No packages configured yet.</div>';
        return;
      }
      const groups = managerOrder
        .map((manager) => ({
          manager,
          rows: rows.filter((entry) => entry.manager === manager)
        }))
        .filter((group) => group.rows.length > 0);

      container.innerHTML = groups.map((group) => `
        <div class="mb-3">
          <div class="mb-2"><span class="chip">${esc(group.manager)}</span></div>
          ${group.rows.map((entry) => {
            const index = rows.indexOf(entry);
            return `
              <div class="domain-item" data-index="${index}">
                <label class="form-check">
                  <input type="checkbox" class="form-check-input package-enabled" data-list="${listType}" data-index="${index}" ${entry.enabled ? 'checked' : ''} ${canEdit ? '' : 'disabled'} />
                  <span class="form-check-label mono">${esc(entry.packageName)}</span>
                </label>
                <div class="domain-item-actions">
                  ${entry.preset ? '<span class="chip">safe default</span>' : '<span class="chip">custom</span>'}
                  ${entry.preset ? '' : `<button class="btn btn-ghost btn-sm package-remove" data-list="${listType}" data-index="${index}" type="button" aria-label="Remove ${esc(entry.packageName)}" ${canEdit ? '' : 'disabled'}>✕</button>`}
                </div>
              </div>
            `;
          }).join('')}
        </div>
      `).join('');
    };

    root.innerHTML = `
      <div class="grid grid-2">
        <div class="card">
          <div class="card-header">
            <h3 class="card-title">Policy Profile</h3>
          </div>
          <div class="card-body">
            <div class="form-group">
              <label class="form-label">Active Profile</label>
              <select id="policy-profile" class="form-select">
                ${profiles.map(profile => `
                  <option value="${esc(profile.id)}" ${effectiveProfileId === profile.id ? 'selected' : ''}>
                    ${esc(profile.label)}
                  </option>
                `).join('')}
              </select>
              <p class="form-hint">${esc(activeProfile?.description || '')}</p>
            </div>

            ${activeProfile?.details?.length ? `
              <div class="alert alert-info mb-4">
                <span class="alert-icon">ℹ</span>
                <div class="alert-content">
                  <div class="alert-title">What This Profile Does</div>
                  <div class="alert-message">
                    <ul style="margin: 8px 0 0 16px;">
                      ${activeProfile.details.map(item => `<li>${esc(item)}</li>`).join('')}
                    </ul>
                  </div>
                </div>
              </div>
            ` : ''}

            ${(!canCustomizeRules || !canCustomizeElevation) ? `
              <div class="alert alert-warning mb-4">
                <span class="alert-icon">⚠</span>
                <div class="alert-content">
                  <div class="alert-title">Profile Locks Are Active</div>
                  <div class="alert-message">
                    ${!canCustomizeRules ? 'Advanced filesystem/execution/persistence controls are locked on this profile.' : ''}
                    ${!canCustomizeElevation ? ' Elevation controls are locked on this profile.' : ''}
                    ${canCreateCustomProfile ? ' Create a Custom Policy if you want full tuning.' : ''}
                  </div>
                </div>
              </div>
            ` : ''}

            <div class="btn-group">
              ${canCreateCustomProfile ? `<button id="policy-create-custom" class="btn btn-ghost btn-sm">Create Custom Policy From Current Settings</button>` : ''}
            </div>

            <div class="mt-4">
              <div class="mb-2"><strong>Version:</strong> <span class="mono">${esc(p.version)}</span></div>
              <div><strong>System Critical Guard:</strong> <span class="chip chip-danger">Always Deny</span></div>
            </div>
          </div>
        </div>

        <div class="card">
          <div class="card-header">
            <h3 class="card-title">Network Controls</h3>
          </div>
          <div class="card-body">
            <div class="form-group">
              <label class="form-check">
                <input type="checkbox" id="toggle-network-deny" class="form-check-input" ${netRules.default_deny !== false ? 'checked' : ''} ${canCustomizeNetwork ? '' : 'disabled'} />
                <span class="form-check-label">Network Default Deny</span>
              </label>
              <p class="form-hint">Editable on all profiles. When enabled, only allowlisted hosts are permitted.</p>
            </div>
            <div class="form-group">
              <label class="form-check">
                <input type="checkbox" id="toggle-network-post-approval" class="form-check-input" ${netRules.require_approval_for_post !== false ? 'checked' : ''} ${canCustomizeNetwork ? '' : 'disabled'} />
                <span class="form-check-label">Require approval for POST requests</span>
              </label>
              <p class="form-hint">Even allowlisted domains can require approval for POST actions (uploads, form submits, API writes).</p>
            </div>
            <button id="policy-save-network" class="btn btn-primary btn-sm" ${canCustomizeNetwork ? '' : 'disabled'}>Save Network Controls</button>
          </div>
        </div>
      </div>

      <div class="card mt-5">
        <div class="card-header">
          <h3 class="card-title">Domain Allowlist</h3>
          <p class="card-description">Manage POST and GET host controls with profile-aware safety checks and approval gates.</p>
        </div>
        <div class="card-body">
          <p class="policy-rule-note">Default: all domain entries start unchecked and invert toggles determine whether checked hosts block or allow traffic, while Agent Ruler still enforces the remaining safety controls.</p>
          <div class="grid grid-2">
            <div class="domain-column">
              <h4 class="card-title">POST Domain Rules</h4>
              <div class="form-group">
                <label class="form-check mb-3">
                  <input type="checkbox" id="toggle-invert-allowlist" class="form-check-input" ${netRules.invert_allowlist !== false ? 'checked' : ''} ${canCustomizeDomains ? '' : 'disabled'} />
                  <span class="form-check-label">Invert (make this a denylist)</span>
                </label>
              </div>
              <div class="form-group">
                <label class="form-label">POST Domain List</label>
                <div id="policy-allowlist-domain-list" class="domain-list" role="list"></div>
                <div class="domain-add-row mt-3">
                  <input id="policy-allowlist-add-input" class="form-input" placeholder="Add domain (example: api.openai.com)" ${canCustomizeDomains ? '' : 'disabled'} />
                  <button id="policy-allowlist-add-btn" class="btn btn-ghost btn-sm" type="button" ${canCustomizeDomains ? '' : 'disabled'}>Add Domain</button>
                </div>
                <p class="policy-rule-note">POST controls outbound write actions (POST/PUT/PATCH/DELETE, uploads, API writes). Allow only trusted domains and prefer manual human POST actions for sensitive writes.</p>
              </div>
              <div class="btn-group domain-actions">
                <button id="policy-reset-allowlist-defaults" class="btn btn-ghost btn-sm" type="button" ${canCustomizeDomains ? '' : 'disabled'}>Reset Defaults</button>
                <button id="policy-save-allowlist" class="btn btn-primary btn-sm" ${canCustomizeDomains ? '' : 'disabled'}>Save POST Domain List</button>
              </div>
            </div>
            <div class="domain-column">
              <h4 class="card-title">GET Domain Rules</h4>
              <div class="form-group">
                <label class="form-check mb-3">
                  <input type="checkbox" id="toggle-invert-denylist" class="form-check-input" ${netRules.invert_denylist !== false ? 'checked' : ''} ${canCustomizeDomains ? '' : 'disabled'} />
                  <span class="form-check-label">Invert (make this a denylist)</span>
                </label>
              </div>
              <div class="form-group">
                <label class="form-label">GET Domain List</label>
                <div id="policy-denylist-domain-list" class="domain-list" role="list"></div>
                <div class="domain-add-row mt-3">
                  <input id="policy-denylist-add-input" class="form-input" placeholder="Add domain (example: docs.example.com)" ${canCustomizeDomains ? '' : 'disabled'} />
                  <button id="policy-denylist-add-btn" class="btn btn-ghost btn-sm" type="button" ${canCustomizeDomains ? '' : 'disabled'}>Add Domain</button>
                </div>
                <p class="policy-rule-note">GET controls browse/read traffic. If <strong>Invert</strong> is enabled, checked domains are the explicit allow-set; if disabled, checked entries are blocked.</p>
              </div>
              <div class="btn-group domain-actions">
                <button id="policy-reset-denylist-defaults" class="btn btn-ghost btn-sm" type="button" ${canCustomizeDomains ? '' : 'disabled'}>Reset Defaults</button>
                <button id="policy-save-denylist" class="btn btn-primary btn-sm" ${canCustomizeDomains ? '' : 'disabled'}>Save GET Domain List</button>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="card mt-5">
        <div class="card-header">
          <h3 class="card-title">Mediated Elevation (sudo mirror)</h3>
        </div>
        <div class="card-body">
          <div class="form-group">
              <label class="form-check">
              <input type="checkbox" id="toggle-elevation-enabled" class="form-check-input" ${elevationRules.enabled !== false ? 'checked' : ''} ${canCustomizeElevation ? '' : 'disabled'} />
                <span class="form-check-label">Enable mediated elevation for install_packages</span>
              </label>
            <p class="form-hint">When enabled, <code>sudo apt install ...</code> requests are converted into approval-gated structured actions.</p>
          </div>
          <div class="form-group">
              <label class="form-check">
              <input type="checkbox" id="toggle-elevation-auth" class="form-check-input" ${elevationRules.require_operator_auth !== false ? 'checked' : ''} ${canCustomizeElevation ? '' : 'disabled'} />
                <span class="form-check-label">Require operator OS authentication</span>
              </label>
            <p class="form-hint">Keeps host-native authentication in the loop before privileged helper execution.</p>
          </div>
          <div class="form-group">
              <label class="form-check">
              <input type="checkbox" id="toggle-elevation-use-allowlist" class="form-check-input" ${elevationUseAllowlist ? 'checked' : ''} ${canCustomizeElevation ? '' : 'disabled'} />
                <span class="form-check-label">Use allowlisted packages</span>
              </label>
            <p class="form-hint">When disabled, allowlist is ignored. Denylist is always enforced.</p>
          </div>
          <div class="grid grid-2">
            <div class="form-group">
              <label class="form-label">Allowlisted Packages</label>
              <label class="form-check mb-3">
                <input type="checkbox" id="policy-allow-packages-enable-all" class="form-check-input" ${canCustomizeElevation ? '' : 'disabled'} />
                <span class="form-check-label">Enable all packages</span>
              </label>
              <div id="policy-allow-package-list" class="domain-list" role="list"></div>
              <div class="domain-add-row mt-3">
                <select id="policy-allow-package-manager" class="form-select" ${canCustomizeElevation ? '' : 'disabled'}>
                  <option value="apt">apt</option>
                  <option value="pip">pip</option>
                  <option value="npm">npm</option>
                  <option value="cargo">cargo</option>
                  <option value="custom">custom</option>
                </select>
                <input id="policy-allow-package-input" class="form-input" placeholder="Add package (example: git)" ${canCustomizeElevation ? '' : 'disabled'} />
                <button id="policy-allow-package-add-btn" class="btn btn-ghost btn-sm" type="button" ${canCustomizeElevation ? '' : 'disabled'}>Add Package</button>
              </div>
            </div>
            <div class="form-group">
              <label class="form-label">Denylisted Packages</label>
              <label class="form-check mb-3">
                <input type="checkbox" id="policy-deny-packages-enable-all" class="form-check-input" ${canCustomizeElevation ? '' : 'disabled'} />
                <span class="form-check-label">Enable all packages</span>
              </label>
              <div id="policy-deny-package-list" class="domain-list" role="list"></div>
              <div class="domain-add-row mt-3">
                <select id="policy-deny-package-manager" class="form-select" ${canCustomizeElevation ? '' : 'disabled'}>
                  <option value="apt">apt</option>
                  <option value="pip">pip</option>
                  <option value="npm">npm</option>
                  <option value="cargo">cargo</option>
                  <option value="custom">custom</option>
                </select>
                <input id="policy-deny-package-input" class="form-input" placeholder="Add package (example: openssh-server)" ${canCustomizeElevation ? '' : 'disabled'} />
                <button id="policy-deny-package-add-btn" class="btn btn-ghost btn-sm" type="button" ${canCustomizeElevation ? '' : 'disabled'}>Add Package</button>
              </div>
              <p class="form-hint">Denylist always wins. Enable packages you do not plan to use, and disable entries before using them intentionally.</p>
            </div>
          </div>
          <button id="policy-save-elevation" class="btn btn-primary btn-sm" ${canCustomizeElevation ? '' : 'disabled'}>Save Elevation Controls</button>
        </div>
      </div>

      <div class="card mt-5">
        <div class="card-header">
          <div>
            <h3 class="card-title">Advanced Rule Customization</h3>
            <p class="card-description">Editable in Coding/Nerd, I DON'T CARE, and Custom profiles.</p>
          </div>
        </div>
        <div class="card-body">
          <div class="grid grid-2">
            <div class="form-group">
              <label class="form-label">Workspace Rule</label>
              <select id="policy-fs-workspace" class="form-select" ${canCustomizeRules ? '' : 'disabled'}>
                <option value="allow" ${fsRules.workspace === 'allow' ? 'selected' : ''}>Allow</option>
                <option value="approval" ${fsRules.workspace === 'approval' ? 'selected' : ''}>Approval</option>
                <option value="deny" ${fsRules.workspace === 'deny' ? 'selected' : ''}>Deny</option>
              </select>
            </div>
            <div class="form-group">
              <label class="form-label">User Data Rule</label>
              <select id="policy-fs-user-data" class="form-select" ${canCustomizeRules ? '' : 'disabled'}>
                <option value="allow" ${fsRules.user_data === 'allow' ? 'selected' : ''}>Allow</option>
                <option value="approval" ${fsRules.user_data === 'approval' ? 'selected' : ''}>Approval</option>
                <option value="deny" ${fsRules.user_data === 'deny' ? 'selected' : ''}>Deny</option>
              </select>
            </div>
            <div class="form-group">
              <label class="form-label">Shared Zone Rule</label>
              <select id="policy-fs-shared" class="form-select" ${canCustomizeRules ? '' : 'disabled'}>
                <option value="allow" ${fsRules.shared === 'allow' ? 'selected' : ''}>Allow</option>
                <option value="approval" ${fsRules.shared === 'approval' ? 'selected' : ''}>Approval</option>
                <option value="deny" ${fsRules.shared === 'deny' ? 'selected' : ''}>Deny</option>
              </select>
            </div>
            <div class="form-group">
              <label class="form-label">Secrets Rule</label>
              <select id="policy-fs-secrets" class="form-select" ${canCustomizeRules ? '' : 'disabled'}>
                <option value="allow" ${fsRules.secrets === 'allow' ? 'selected' : ''}>Allow</option>
                <option value="approval" ${fsRules.secrets === 'approval' ? 'selected' : ''}>Approval</option>
                <option value="deny" ${fsRules.secrets === 'deny' ? 'selected' : ''}>Deny</option>
              </select>
            </div>
            <div class="form-group">
              <label class="form-check">
                <input type="checkbox" id="policy-exec-workspace" class="form-check-input" ${execRules.deny_workspace_exec !== false ? 'checked' : ''} ${canCustomizeRules ? '' : 'disabled'} />
                <span class="form-check-label">Deny Execution From Workspace</span>
              </label>
            </div>
            <div class="form-group">
              <label class="form-check">
                <input type="checkbox" id="policy-exec-tmp" class="form-check-input" ${execRules.deny_tmp_exec !== false ? 'checked' : ''} ${canCustomizeRules ? '' : 'disabled'} />
                <span class="form-check-label">Deny Execution From Temp Paths</span>
              </label>
            </div>
            <div class="form-group">
              <label class="form-check">
                <input type="checkbox" id="policy-exec-quarantine" class="form-check-input" ${execRules.quarantine_on_download_exec_chain !== false ? 'checked' : ''} ${canCustomizeRules ? '' : 'disabled'} />
                <span class="form-check-label">Quarantine Download->Exec Chains</span>
              </label>
            </div>
            <div class="form-group">
              <label class="form-check">
                <input type="checkbox" id="policy-persistence-autostart" class="form-check-input" ${persistRules.deny_autostart !== false ? 'checked' : ''} ${canCustomizeRules ? '' : 'disabled'} />
                <span class="form-check-label">Deny Autostart Persistence</span>
              </label>
            </div>
            <div class="form-group">
              <label class="form-label">Mass Delete Threshold</label>
              <input id="policy-mass-delete-threshold" type="number" min="1" class="form-input" value="${massDeleteThreshold}" ${canCustomizeRules ? '' : 'disabled'} />
              <p class="form-hint">Represents the number of files in a single delete operation that triggers an approval requirement.</p>
            </div>
          </div>
          <button id="policy-save-rules" class="btn btn-secondary btn-sm" ${canCustomizeRules ? '' : 'disabled'}>Save Advanced Rules</button>
        </div>
      </div>

      <div class="card mt-5">
        <div class="card-header">
          <h3 class="card-title">Current Zone Configuration</h3>
        </div>
        <div class="card-body">
          <div class="table-container">
            <table class="table">
              <thead>
                <tr>
                  <th>Zone</th>
                  <th>Name</th>
                  <th>Active Rule</th>
                  <th>Description</th>
                </tr>
              </thead>
              <tbody>
                <tr>
                  <td><span class="chip chip-success">0</span></td>
                  <td>Workspace</td>
                  <td>${esc(dispLabel(fsRules.workspace))}</td>
                  <td>Agent working directory.</td>
                </tr>
                <tr>
                  <td><span class="chip chip-primary">1</span></td>
                  <td>User Data</td>
                  <td>${esc(dispLabel(fsRules.user_data))}</td>
                  <td>User documents and app config.</td>
                </tr>
                <tr>
                  <td><span class="chip chip-warning">2</span></td>
                  <td>Shared Zone</td>
                  <td>${esc(dispLabel(fsRules.shared))}</td>
                  <td>Export staging and approval boundary.</td>
                </tr>
                <tr>
                  <td><span class="chip chip-danger">3</span></td>
                  <td>System Critical</td>
                  <td><strong>Deny (Enforced)</strong></td>
                  <td>System binaries and host-critical configuration.</td>
                </tr>
                <tr>
                  <td><span class="chip chip-danger">4</span></td>
                  <td>Secrets</td>
                  <td>${esc(dispLabel(fsRules.secrets))}</td>
                  <td>Credentials, keys, and sensitive material.</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>
      </div>
    `;

    // Render domain lists
    renderDomainListItems('policy-allowlist-domain-list', allowlistRows, 'allowlist', canCustomizeDomains);
    renderDomainListItems('policy-denylist-domain-list', denylistRows, 'denylist', canCustomizeDomains);
    const syncPackageBulkToggle = (toggleId, rows) => {
      const toggle = document.getElementById(toggleId);
      if (!(toggle instanceof HTMLInputElement)) return;
      if (!rows.length) {
        toggle.checked = false;
        toggle.indeterminate = false;
        toggle.disabled = true;
        return;
      }
      toggle.disabled = false;
      const enabledCount = rows.filter((entry) => entry.enabled).length;
      toggle.checked = enabledCount === rows.length;
      toggle.indeterminate = enabledCount > 0 && enabledCount < rows.length;
    };

    const renderPackageLists = () => {
      renderPackageListItems('policy-allow-package-list', allowPackageRows, 'allow-packages', canCustomizeElevation);
      renderPackageListItems('policy-deny-package-list', denyPackageRows, 'deny-packages', canCustomizeElevation);
      syncPackageBulkToggle('policy-allow-packages-enable-all', allowPackageRows);
      syncPackageBulkToggle('policy-deny-packages-enable-all', denyPackageRows);
    };
    renderPackageLists();

    document.getElementById('policy-profile').addEventListener('change', async (e) => {
      try {
        const updated = await api('/api/policy/toggles', {
          method: 'POST',
          body: { profile: e.target.value }
        });
        state.policy = updated;
        toast('Policy profile updated', 'success');
        await refreshStatus();
        await renderPolicy(root);
      } catch (err) {
        toast(`Failed to update profile: ${err.message}`, 'error');
      }
    });

    const createCustomBtn = document.getElementById('policy-create-custom');
    if (createCustomBtn) {
      createCustomBtn.addEventListener('click', async () => {
        try {
          const updated = await api('/api/policy/toggles', {
            method: 'POST',
            body: { create_custom_profile: true }
          });
          state.policy = updated;
          toast('Custom Policy created from current settings', 'success');
          await refreshStatus();
          await renderPolicy(root);
        } catch (err) {
          toast(`Failed to create custom profile: ${err.message}`, 'error');
        }
      });
    }

    // Setup domain list event handlers
    const setupDomainListEvents = (listId, rows, listType) => {
      const list = document.getElementById(listId);
      if (!list) return;
      
      list.addEventListener('change', (event) => {
        const target = event.target;
        if (!(target instanceof HTMLInputElement)) return;
        if (!target.classList.contains('domain-enabled')) return;
        const idx = Number(target.dataset.index);
        if (!Number.isInteger(idx) || idx < 0 || idx >= rows.length) return;
        rows[idx].enabled = !!target.checked;
      });

      list.addEventListener('click', (event) => {
        const target = event.target;
        if (!(target instanceof HTMLElement)) return;
        const button = target.closest('.domain-remove');
        if (!button) return;
        const idx = Number(button.dataset.index);
        if (!Number.isInteger(idx) || idx < 0 || idx >= rows.length) return;
        rows.splice(idx, 1);
        renderDomainListItems(listId, rows, listType, canCustomizeDomains);
      });
    };

    setupDomainListEvents('policy-allowlist-domain-list', allowlistRows, 'allowlist');
    setupDomainListEvents('policy-denylist-domain-list', denylistRows, 'denylist');

    const setupPackageListEvents = (listId, rows, listType, bulkToggleId) => {
      const list = document.getElementById(listId);
      if (!list) return;

      list.addEventListener('change', (event) => {
        const target = event.target;
        if (!(target instanceof HTMLInputElement)) return;
        if (!target.classList.contains('package-enabled')) return;
        const idx = Number(target.dataset.index);
        if (!Number.isInteger(idx) || idx < 0 || idx >= rows.length) return;
        rows[idx].enabled = !!target.checked;
        syncPackageBulkToggle(bulkToggleId, rows);
      });

      list.addEventListener('click', (event) => {
        const target = event.target;
        if (!(target instanceof HTMLElement)) return;
        const button = target.closest('.package-remove');
        if (!button) return;
        const idx = Number(button.dataset.index);
        if (!Number.isInteger(idx) || idx < 0 || idx >= rows.length) return;
        rows.splice(idx, 1);
        renderPackageListItems(listId, rows, listType, canCustomizeElevation);
        syncPackageBulkToggle(bulkToggleId, rows);
      });
    };
    setupPackageListEvents('policy-allow-package-list', allowPackageRows, 'allow-packages', 'policy-allow-packages-enable-all');
    setupPackageListEvents('policy-deny-package-list', denyPackageRows, 'deny-packages', 'policy-deny-packages-enable-all');

    // Setup add domain handlers
    const setupAddDomainHandler = (inputId, btnId, rows, listId, listType) => {
      const input = document.getElementById(inputId);
      const btn = document.getElementById(btnId);
      if (!input || !btn) return;
      
      const addDomain = () => {
        const normalized = normalizeDomainHost(input.value);
        if (!normalized) {
          toast('Enter a valid domain host (example: api.example.com)', 'warning');
          return;
        }
        const existing = rows.find(item => item.host === normalized);
        if (existing) {
          existing.enabled = true;
          input.value = '';
          renderDomainListItems(listId, rows, listType, canCustomizeDomains);
          return;
        }
        rows.push({ host: normalized, preset: false, enabled: true });
        input.value = '';
        renderDomainListItems(listId, rows, listType, canCustomizeDomains);
      };
      
      btn.addEventListener('click', addDomain);
      input.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') {
          event.preventDefault();
          addDomain();
        }
      });
    };

    setupAddDomainHandler('policy-allowlist-add-input', 'policy-allowlist-add-btn', allowlistRows, 'policy-allowlist-domain-list', 'allowlist');
    setupAddDomainHandler('policy-denylist-add-input', 'policy-denylist-add-btn', denylistRows, 'policy-denylist-domain-list', 'denylist');

    const resetDomainRows = (rows, defaults) => {
      rows.splice(0, rows.length);
      defaults.forEach((host) => {
        const normalized = normalizeDomainHost(host);
        if (!normalized) return;
        rows.push({ host: normalized, preset: true, enabled: true });
      });
    };

    const resetAllowBtn = document.getElementById('policy-reset-allowlist-defaults');
    if (resetAllowBtn) {
      resetAllowBtn.addEventListener('click', () => {
        resetDomainRows(allowlistRows, safeDomainPresets);
        renderDomainListItems('policy-allowlist-domain-list', allowlistRows, 'allowlist', canCustomizeDomains);
        toast('POST domain defaults restored', 'success');
      });
    }

    const resetDenyBtn = document.getElementById('policy-reset-denylist-defaults');
    if (resetDenyBtn) {
      resetDenyBtn.addEventListener('click', () => {
        resetDomainRows(denylistRows, denylistPresets);
        renderDomainListItems('policy-denylist-domain-list', denylistRows, 'denylist', canCustomizeDomains);
        toast('GET domain defaults restored', 'success');
      });
    }

    const setupPackageBulkToggle = (toggleId, rows, listId, listType) => {
      const toggle = document.getElementById(toggleId);
      if (!(toggle instanceof HTMLInputElement)) return;
      toggle.addEventListener('change', () => {
        rows.forEach((entry) => {
          entry.enabled = !!toggle.checked;
        });
        renderPackageListItems(listId, rows, listType, canCustomizeElevation);
        syncPackageBulkToggle(toggleId, rows);
      });
    };

    const setupAddPackageHandler = (inputId, managerId, btnId, rows, listId, listType, toggleId) => {
      const input = document.getElementById(inputId);
      const managerSelect = document.getElementById(managerId);
      const btn = document.getElementById(btnId);
      if (!input || !managerSelect || !btn) return;

      const addPackage = () => {
        const normalizedPkg = normalizePackageName(input.value);
        if (!normalizedPkg) {
          toast('Enter a valid package name (example: requests)', 'warning');
          return;
        }
        const selectedManager = normalizeManager(managerSelect.value);
        const existing = rows.find((item) => item.packageName === normalizedPkg);
        if (existing) {
          existing.enabled = true;
          if (existing.manager === 'custom' && selectedManager !== 'custom') {
            existing.manager = selectedManager;
          }
          input.value = '';
          renderPackageListItems(listId, rows, listType, canCustomizeElevation);
          syncPackageBulkToggle(toggleId, rows);
          return;
        }
        rows.push({
          manager: selectedManager,
          packageName: normalizedPkg,
          preset: false,
          enabled: true
        });
        rows.sort((a, b) => {
          const managerDelta = managerOrder.indexOf(a.manager) - managerOrder.indexOf(b.manager);
          if (managerDelta !== 0) return managerDelta;
          return a.packageName.localeCompare(b.packageName);
        });
        input.value = '';
        renderPackageListItems(listId, rows, listType, canCustomizeElevation);
        syncPackageBulkToggle(toggleId, rows);
      };

      btn.addEventListener('click', addPackage);
      input.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') {
          event.preventDefault();
          addPackage();
        }
      });
    };

    setupPackageBulkToggle('policy-allow-packages-enable-all', allowPackageRows, 'policy-allow-package-list', 'allow-packages');
    setupPackageBulkToggle('policy-deny-packages-enable-all', denyPackageRows, 'policy-deny-package-list', 'deny-packages');
    setupAddPackageHandler('policy-allow-package-input', 'policy-allow-package-manager', 'policy-allow-package-add-btn', allowPackageRows, 'policy-allow-package-list', 'allow-packages', 'policy-allow-packages-enable-all');
    setupAddPackageHandler('policy-deny-package-input', 'policy-deny-package-manager', 'policy-deny-package-add-btn', denyPackageRows, 'policy-deny-package-list', 'deny-packages', 'policy-deny-packages-enable-all');

    // Save basic network controls
    document.getElementById('policy-save-network').addEventListener('click', async () => {
      try {
        const updated = await api('/api/policy/toggles', {
          method: 'POST',
          body: {
            network_default_deny: document.getElementById('toggle-network-deny').checked,
            network_require_approval_for_post: document.getElementById('toggle-network-post-approval').checked
          }
        });
        state.policy = updated;
        toast('Network controls updated', 'success');
        await refreshStatus();
        await renderPolicy(root);
      } catch (err) {
        toast(`Failed to update network controls: ${err.message}`, 'error');
      }
    });

    // Save allowlist (POST domains)
    document.getElementById('policy-save-allowlist').addEventListener('click', async () => {
      try {
        const allowlistHosts = allowlistRows
          .filter(item => item.enabled)
          .map(item => item.host);

        const updated = await api('/api/policy/toggles', {
          method: 'POST',
          body: {
            network_allowlist_hosts: allowlistHosts,
            network_invert_allowlist: document.getElementById('toggle-invert-allowlist').checked
          }
        });
        state.policy = updated;
        toast('POST domain list updated', 'success');
        await refreshStatus();
        await renderPolicy(root);
      } catch (err) {
        toast(`Failed to update POST domain list: ${err.message}`, 'error');
      }
    });

    // Save denylist (GET domains)
    document.getElementById('policy-save-denylist').addEventListener('click', async () => {
      try {
        const denylistHosts = denylistRows
          .filter(item => item.enabled)
          .map(item => item.host);

        const updated = await api('/api/policy/toggles', {
          method: 'POST',
          body: {
            network_denylist_hosts: denylistHosts,
            network_invert_denylist: document.getElementById('toggle-invert-denylist').checked
          }
        });
        state.policy = updated;
        toast('GET domain list updated', 'success');
        await refreshStatus();
        await renderPolicy(root);
      } catch (err) {
        toast(`Failed to update GET domain list: ${err.message}`, 'error');
      }
    });

    document.getElementById('policy-save-elevation').addEventListener('click', async () => {
      try {
        const collectEnabledPackages = (rows) => {
          const packages = [];
          const seen = new Set();
          rows.forEach((entry) => {
            if (!entry.enabled) return;
            const pkg = normalizePackageName(entry.packageName);
            if (!pkg || seen.has(pkg)) return;
            seen.add(pkg);
            packages.push(pkg);
          });
          return packages;
        };
        const allowedPackages = collectEnabledPackages(allowPackageRows);
        const deniedPackages = collectEnabledPackages(denyPackageRows);

        const updated = await api('/api/policy/toggles', {
          method: 'POST',
          body: {
            elevation_enabled: document.getElementById('toggle-elevation-enabled').checked,
            elevation_require_operator_auth: document.getElementById('toggle-elevation-auth').checked,
            elevation_use_allowlist: document.getElementById('toggle-elevation-use-allowlist').checked,
            elevation_allowed_packages: allowedPackages,
            elevation_denied_packages: deniedPackages
          }
        });
        state.policy = updated;
        toast('Elevation controls updated', 'success');
        await refreshStatus();
        await renderPolicy(root);
      } catch (err) {
        toast(`Failed to update elevation controls: ${err.message}`, 'error');
      }
    });

    document.getElementById('policy-save-rules').addEventListener('click', async () => {
      try {
        const massDeleteThresholdInput = document.getElementById('policy-mass-delete-threshold')?.value;
        const payload = {
          filesystem_workspace: document.getElementById('policy-fs-workspace').value,
          filesystem_user_data: document.getElementById('policy-fs-user-data').value,
          filesystem_shared: document.getElementById('policy-fs-shared').value,
          filesystem_secrets: document.getElementById('policy-fs-secrets').value,
          execution_deny_workspace_exec: document.getElementById('policy-exec-workspace').checked,
          execution_deny_tmp_exec: document.getElementById('policy-exec-tmp').checked,
          execution_quarantine_on_download_exec_chain: document.getElementById('policy-exec-quarantine').checked,
          persistence_deny_autostart: document.getElementById('policy-persistence-autostart').checked,
          safeguards_mass_delete_threshold: toSafePositiveInt(massDeleteThresholdInput, 40)
        };

        const updated = await api('/api/policy/toggles', {
          method: 'POST',
          body: payload
        });
        state.policy = updated;
        toast('Advanced policy rules updated', 'success');
        await refreshStatus();
        await renderPolicy(root);
      } catch (err) {
        toast(`Failed to update advanced rules: ${err.message}`, 'error');
      }
    });
  }

  // ============================================
  // Receipts Page
  // ============================================
