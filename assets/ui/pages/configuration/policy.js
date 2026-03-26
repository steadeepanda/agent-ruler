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
      <div class="settings-container">
        <div class="settings-header" style="margin-bottom: var(--space-6); padding-bottom: var(--space-6); border-bottom: 1px solid var(--content-border);">
          <div>
            <h2 class="settings-title">Policy Configuration</h2>
            <p class="settings-description">Manage security boundaries and execution rules.</p>
          </div>
          <div style="display: flex; gap: var(--space-2);">
             <button id="policy-reset-all" class="btn btn-ghost btn-sm" style="display: none;">Discard Changes</button>
             <button id="policy-save-all" class="btn btn-primary btn-sm" disabled>Save Changes</button>
          </div>
        </div>

        <div class="settings-section" style="margin-bottom: var(--space-6);">
          <div class="settings-section-header">
            <h3>Policy Profile</h3>
            <p>Active security baseline and capability locks.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row">
              <label class="form-label">Active Profile</label>
              <select id="policy-profile" class="form-select" style="max-width: 400px;">
                ${profiles.map(profile => `
                  <option value="${esc(profile.id)}" ${effectiveProfileId === profile.id ? 'selected' : ''}>
                    ${esc(profile.label)}
                  </option>
                `).join('')}
              </select>
              <p class="form-hint">${esc(activeProfile?.description || '')}</p>
            </div>

            ${activeProfile?.details?.length ? `
              <div class="alert alert-info mt-4">
                <div class="alert-icon">
                  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>
                </div>
                <div class="alert-content">
                  <h4 class="alert-title">What This Profile Does</h4>
                  <ul class="alert-message" style="margin: 0; padding-left: 20px;">
                    ${activeProfile.details.map(item => `<li>${esc(item)}</li>`).join('')}
                  </ul>
                </div>
              </div>
            ` : ''}

            ${(!canCustomizeRules || !canCustomizeElevation) ? `
              <div class="alert alert-warning mt-4">
                <div class="alert-icon">
                  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></svg>
                </div>
                <div class="alert-content">
                  <h4 class="alert-title">Profile Locks Are Active</h4>
                  <p class="alert-message" style="margin: 0;">
                    ${!canCustomizeRules ? 'Advanced filesystem/execution controls are locked. ' : ''}
                    ${!canCustomizeElevation ? 'Elevation controls are locked. ' : ''}
                    ${canCreateCustomProfile ? 'Create a Custom Policy for full tuning.' : ''}
                  </p>
                </div>
              </div>
            ` : ''}

            ${canCreateCustomProfile ? `
              <div class="settings-row mt-4">
                <button id="policy-create-custom" class="btn btn-secondary" style="align-self: flex-start;">Create Custom Policy From Current Settings</button>
              </div>
            ` : ''}
            
          </div>
        </div>

        <div class="panel-tabs mb-6" id="policy-main-tabs" role="tablist">
          <button type="button" class="panel-tab active" data-policy-tab="status">Status</button>
          <button type="button" class="panel-tab" data-policy-tab="network">Network</button>
          <button type="button" class="panel-tab" data-policy-tab="domains">Domains</button>
          <button type="button" class="panel-tab" data-policy-tab="elevation">Elevation</button>
          <button type="button" class="panel-tab" data-policy-tab="advanced">Advanced</button>
        </div>

        <div id="policy-tab-network" class="settings-section" style="display: none;">
          <div class="settings-section-header">
            <h3>Network Controls</h3>
            <p>Global network restrictions spanning all domains.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row">
              <label class="form-switch">
                <input type="checkbox" id="toggle-network-deny" class="form-switch-input" ${netRules.default_deny !== false ? 'checked' : ''} ${canCustomizeNetwork ? '' : 'disabled'} />
                <div class="form-switch-text">
                  <span class="form-switch-label">Network Default Deny</span>
                  <span class="form-switch-description">Editable on all profiles. When enabled, only allowlisted hosts are permitted.</span>
                </div>
              </label>
            </div>
            <div class="settings-row mt-4">
              <label class="form-switch">
                <input type="checkbox" id="toggle-network-post-approval" class="form-switch-input" ${netRules.require_approval_for_post !== false ? 'checked' : ''} ${canCustomizeNetwork ? '' : 'disabled'} />
                <div class="form-switch-text">
                  <span class="form-switch-label">Require approval for POST requests</span>
                  <span class="form-switch-description">Even allowlisted domains can require approval for POST actions (uploads, form submits, API writes).</span>
                </div>
              </label>
            </div>

          </div>
        </div>

        <div id="policy-tab-domains" class="settings-section" style="display: none;">
          <div class="settings-section-header">
            <h3>Domain Boundary</h3>
            <p>Manage discrete domain access. Defaults start unchecked; toggle behavior implies block or allow based on inversion.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row-split" style="gap: var(--space-8); align-items: flex-start; flex-wrap: wrap;">
              
              <!-- POST Domain Rules -->
              <div class="settings-row" style="flex: 1; min-width: 0;">
                <h4 style="font-size: 1.05rem; font-weight: 600; color: var(--text-primary); margin-bottom: var(--space-4);">POST Form/API Traffic</h4>
                <div class="settings-row mb-4">
                  <label class="form-switch">
                    <input type="checkbox" id="toggle-invert-allowlist" class="form-switch-input" ${netRules.invert_allowlist !== false ? 'checked' : ''} ${canCustomizeDomains ? '' : 'disabled'} />
                    <div class="form-switch-text">
                      <span class="form-switch-label">Invert to Denylist</span>
                    </div>
                  </label>
                </div>
                
                <div id="policy-allowlist-domain-list" class="domain-list mb-4" role="list"></div>
                
                <div style="display: flex; gap: var(--space-2); margin-bottom: var(--space-4);">
                  <input id="policy-allowlist-add-input" class="form-input" placeholder="Add domain (e.g., api.openai.com)" ${canCustomizeDomains ? '' : 'disabled'} />
                  <button id="policy-allowlist-add-btn" class="btn btn-secondary" type="button" ${canCustomizeDomains ? '' : 'disabled'}>Add</button>
                </div>
                
                <p class="form-hint mb-4">POST controls outbound write actions/uploads.</p>
                
                <div style="display: flex; gap: var(--space-2);">
                  <button id="policy-reset-allowlist-defaults" class="btn btn-sm btn-outline" type="button" ${canCustomizeDomains ? '' : 'disabled'}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="margin-right: 4px;"><path d="M21 2v6h-6"/><path d="M3 12a9 9 0 1 0 2.6-6.4L2 9"/></svg> Reset Defaults</button>
                </div>
              </div>
              
              <!-- GET Domain Rules -->
              <div class="settings-row" style="flex: 1; min-width: 0;">
                <h4 style="font-size: 1.05rem; font-weight: 600; color: var(--text-primary); margin-bottom: var(--space-4);">GET Browse Traffic</h4>
                <div class="settings-row mb-4">
                  <label class="form-switch">
                    <input type="checkbox" id="toggle-invert-denylist" class="form-switch-input" ${netRules.invert_denylist !== false ? 'checked' : ''} ${canCustomizeDomains ? '' : 'disabled'} />
                    <div class="form-switch-text">
                      <span class="form-switch-label">Invert to Denylist</span>
                    </div>
                  </label>
                </div>
                
                <div id="policy-denylist-domain-list" class="domain-list mb-4" role="list"></div>
                
                <div style="display: flex; gap: var(--space-2); margin-bottom: var(--space-4);">
                  <input id="policy-denylist-add-input" class="form-input" placeholder="Add domain (e.g., docs.example.com)" ${canCustomizeDomains ? '' : 'disabled'} />
                  <button id="policy-denylist-add-btn" class="btn btn-secondary" type="button" ${canCustomizeDomains ? '' : 'disabled'}>Add</button>
                </div>
                
                <p class="form-hint mb-4">GET controls standard site browsing/reads.</p>
                
                <div style="display: flex; gap: var(--space-2);">
                  <button id="policy-reset-denylist-defaults" class="btn btn-sm btn-outline" type="button" ${canCustomizeDomains ? '' : 'disabled'}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="margin-right: 4px;"><path d="M21 2v6h-6"/><path d="M3 12a9 9 0 1 0 2.6-6.4L2 9"/></svg> Reset Defaults</button>
                </div>
              </div>
              
              </div>
            </div>
          </div>
        </div>

        <div id="policy-tab-elevation" class="settings-section" style="display: none;">
          <div class="settings-section-header">
            <h3>Mediated Elevation</h3>
            <p>Controls how sudo and package manger commands are governed.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row">
              <label class="form-switch">
                <input type="checkbox" id="toggle-elevation-enabled" class="form-switch-input" ${elevationRules.enabled !== false ? 'checked' : ''} ${canCustomizeElevation ? '' : 'disabled'} />
                <div class="form-switch-text">
                  <span class="form-switch-label">Enable mediated elevation</span>
                  <span class="form-switch-description">Converts <code>sudo apt install</code> requests into approval-gated actions.</span>
                </div>
              </label>
            </div>
            <div class="settings-row mt-4">
              <label class="form-switch">
                <input type="checkbox" id="toggle-elevation-auth" class="form-switch-input" ${elevationRules.require_operator_auth !== false ? 'checked' : ''} ${canCustomizeElevation ? '' : 'disabled'} />
                <div class="form-switch-text">
                  <span class="form-switch-label">Require operator OS auth</span>
                  <span class="form-switch-description">Keeps host-native authentication in the loop before helper execution.</span>
                </div>
              </label>
            </div>
            <div class="settings-row mt-4">
              <label class="form-switch">
                <input type="checkbox" id="toggle-elevation-use-allowlist" class="form-switch-input" ${elevationUseAllowlist ? 'checked' : ''} ${canCustomizeElevation ? '' : 'disabled'} />
                <div class="form-switch-text">
                  <span class="form-switch-label">Use allowlisted packages</span>
                  <span class="form-switch-description">When disabled, allowlist is ignored. Denylist is always enforced.</span>
                </div>
              </label>
            </div>
            
            <div class="settings-row-split mt-6" style="gap: var(--space-8); align-items: flex-start; border-top: 1px solid var(--content-border); padding-top: var(--space-6);">
              <div class="settings-row" style="flex: 1; min-width: 0;">
                <h4 style="font-size: 1.05rem; font-weight: 600; color: var(--text-primary); margin-bottom: var(--space-4);">Allowlisted Packages</h4>
                <div class="settings-row mb-4">
                  <label class="form-switch">
                    <input type="checkbox" id="policy-allow-packages-enable-all" class="form-switch-input" ${canCustomizeElevation ? '' : 'disabled'} />
                    <div class="form-switch-text">
                      <span class="form-switch-label" style="font-size: 0.9rem;">Enable all</span>
                    </div>
                  </label>
                </div>
                
                <div id="policy-allow-package-list" class="domain-list mb-4" style="max-height: 400px; overflow-y: auto;" role="list"></div>
                
                <div style="display: flex; gap: var(--space-2); margin-bottom: var(--space-4);">
                  <select id="policy-allow-package-manager" class="form-select" style="width: auto;" ${canCustomizeElevation ? '' : 'disabled'}>
                    <option value="apt">apt</option>
                    <option value="pip">pip</option>
                    <option value="npm">npm</option>
                    <option value="cargo">cargo</option>
                    <option value="custom">custom</option>
                  </select>
                  <input id="policy-allow-package-input" class="form-input" style="flex: 1;" placeholder="e.g., git" ${canCustomizeElevation ? '' : 'disabled'} />
                  <button id="policy-allow-package-add-btn" class="btn btn-secondary" type="button" ${canCustomizeElevation ? '' : 'disabled'}>Add</button>
                </div>
              </div>
              
              <div class="settings-row" style="flex: 1; min-width: 0;">
                <h4 style="font-size: 1.05rem; font-weight: 600; color: var(--text-primary); margin-bottom: var(--space-4);">Denylisted Packages</h4>
                <div class="settings-row mb-4">
                  <label class="form-switch">
                    <input type="checkbox" id="policy-deny-packages-enable-all" class="form-switch-input" ${canCustomizeElevation ? '' : 'disabled'} />
                    <div class="form-switch-text">
                      <span class="form-switch-label" style="font-size: 0.9rem;">Enable all</span>
                    </div>
                  </label>
                </div>
                
                <div id="policy-deny-package-list" class="domain-list mb-4" style="max-height: 400px; overflow-y: auto;" role="list"></div>
                
                <div style="display: flex; gap: var(--space-2); margin-bottom: var(--space-4);">
                  <select id="policy-deny-package-manager" class="form-select" style="width: auto;" ${canCustomizeElevation ? '' : 'disabled'}>
                    <option value="apt">apt</option>
                    <option value="pip">pip</option>
                    <option value="npm">npm</option>
                    <option value="cargo">cargo</option>
                    <option value="custom">custom</option>
                  </select>
                  <input id="policy-deny-package-input" class="form-input" style="flex: 1;" placeholder="e.g., openssh-server" ${canCustomizeElevation ? '' : 'disabled'} />
                  <button id="policy-deny-package-add-btn" class="btn btn-secondary" type="button" ${canCustomizeElevation ? '' : 'disabled'}>Add</button>
                </div>
                <p class="form-hint" style="margin-top: 0;">Denylist always wins. Enable packages you do not plan to use, and disable entries before using them intentionally.</p>
              </div>
            </div>

          </div>
        </div>

        <div id="policy-tab-advanced" class="settings-section" style="display: none;">
          <div class="settings-section-header">
            <h3>Advanced Rules</h3>
            <p>Tunable paths and execution layers for Nerd / Custom profiles.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row-split" style="gap: var(--space-6);">
              <div class="settings-row" style="flex:1;">
                <label class="form-label">Workspace Rule</label>
                <select id="policy-fs-workspace" class="form-select" ${canCustomizeRules ? '' : 'disabled'}>
                  <option value="allow" ${fsRules.workspace === 'allow' ? 'selected' : ''}>Allow</option>
                  <option value="approval" ${fsRules.workspace === 'approval' ? 'selected' : ''}>Approval</option>
                  <option value="deny" ${fsRules.workspace === 'deny' ? 'selected' : ''}>Deny</option>
                </select>
              </div>
              <div class="settings-row" style="flex:1;">
                <label class="form-label">User Data Rule</label>
                <select id="policy-fs-user-data" class="form-select" ${canCustomizeRules ? '' : 'disabled'}>
                  <option value="allow" ${fsRules.user_data === 'allow' ? 'selected' : ''}>Allow</option>
                  <option value="approval" ${fsRules.user_data === 'approval' ? 'selected' : ''}>Approval</option>
                  <option value="deny" ${fsRules.user_data === 'deny' ? 'selected' : ''}>Deny</option>
                </select>
              </div>
            </div>
            
            <div class="settings-row-split mt-4" style="gap: var(--space-6);">
              <div class="settings-row" style="flex:1;">
                <label class="form-label">Shared Zone Rule</label>
                <select id="policy-fs-shared" class="form-select" ${canCustomizeRules ? '' : 'disabled'}>
                  <option value="allow" ${fsRules.shared === 'allow' ? 'selected' : ''}>Allow</option>
                  <option value="approval" ${fsRules.shared === 'approval' ? 'selected' : ''}>Approval</option>
                  <option value="deny" ${fsRules.shared === 'deny' ? 'selected' : ''}>Deny</option>
                </select>
              </div>
              <div class="settings-row" style="flex:1;">
                <label class="form-label">Secrets Rule</label>
                <select id="policy-fs-secrets" class="form-select" ${canCustomizeRules ? '' : 'disabled'}>
                  <option value="allow" ${fsRules.secrets === 'allow' ? 'selected' : ''}>Allow</option>
                  <option value="approval" ${fsRules.secrets === 'approval' ? 'selected' : ''}>Approval</option>
                  <option value="deny" ${fsRules.secrets === 'deny' ? 'selected' : ''}>Deny</option>
                </select>
              </div>
            </div>
            
            <div class="settings-row mt-6">
              <label class="form-switch">
                <input type="checkbox" id="policy-exec-workspace" class="form-switch-input" ${execRules.deny_workspace_exec !== false ? 'checked' : ''} ${canCustomizeRules ? '' : 'disabled'} />
                <div class="form-switch-text">
                  <span class="form-switch-label">Deny Execution From Workspace</span>
                </div>
              </label>
            </div>
            <div class="settings-row mt-3">
              <label class="form-switch">
                <input type="checkbox" id="policy-exec-tmp" class="form-switch-input" ${execRules.deny_tmp_exec !== false ? 'checked' : ''} ${canCustomizeRules ? '' : 'disabled'} />
                <div class="form-switch-text">
                  <span class="form-switch-label">Deny Execution From Temp Paths</span>
                </div>
              </label>
            </div>
            <div class="settings-row mt-3">
              <label class="form-switch">
                <input type="checkbox" id="policy-exec-quarantine" class="form-switch-input" ${execRules.quarantine_on_download_exec_chain !== false ? 'checked' : ''} ${canCustomizeRules ? '' : 'disabled'} />
                <div class="form-switch-text">
                  <span class="form-switch-label">Quarantine Download->Exec Chains</span>
                </div>
              </label>
            </div>
            <div class="settings-row mt-3">
              <label class="form-switch">
                <input type="checkbox" id="policy-persistence-autostart" class="form-switch-input" ${persistRules.deny_autostart !== false ? 'checked' : ''} ${canCustomizeRules ? '' : 'disabled'} />
                <div class="form-switch-text">
                  <span class="form-switch-label">Deny Autostart Persistence</span>
                </div>
              </label>
            </div>
            
            <div class="settings-row-split mt-6">
              <div class="settings-row">
                <label class="form-label">Mass Delete Threshold</label>
                <input id="policy-mass-delete-threshold" type="number" min="1" class="form-input" style="max-width: 150px;" value="${massDeleteThreshold}" ${canCustomizeRules ? '' : 'disabled'} />
                <p class="form-hint">Number of files in a single delete operation that triggers an approval requirement.</p>
              </div>
            </div>
            

          </div>
        </div>

        <div id="policy-tab-status" class="settings-section">
          <div class="settings-section-header">
            <h3>Active Boundaries</h3>
            <p>Summary of zone states.</p>
          </div>
          <div class="settings-section-content">
            <div class="table-container" style="border: 1px solid var(--content-border); border-radius: var(--radius-lg); overflow: hidden;">
              <table class="table" style="margin: 0; border: none;">
                <thead>
                  <tr>
                    <th>Zone</th>
                    <th>Name</th>
                    <th>Active Rule</th>
                    <th>Description</th>
                  </tr>
                </thead>
                <tbody style="border-top: 1px solid var(--content-border);">
                  <tr>
                    <td><span class="chip chip-success">0</span></td>
                    <td style="font-weight: 500;">Workspace</td>
                    <td><span class="chip" style="background: var(--content-bg-alt);">${esc(dispLabel(fsRules.workspace))}</span></td>
                    <td class="text-muted">Agent working directory.</td>
                  </tr>
                  <tr>
                    <td><span class="chip chip-primary">1</span></td>
                    <td style="font-weight: 500;">User Data</td>
                    <td><span class="chip" style="background: var(--content-bg-alt);">${esc(dispLabel(fsRules.user_data))}</span></td>
                    <td class="text-muted">User documents and app config.</td>
                  </tr>
                  <tr>
                    <td><span class="chip chip-warning">2</span></td>
                    <td style="font-weight: 500;">Shared Zone</td>
                    <td><span class="chip" style="background: var(--content-bg-alt);">${esc(dispLabel(fsRules.shared))}</span></td>
                    <td class="text-muted">Export staging and approval boundary.</td>
                  </tr>
                  <tr style="background: var(--danger-bg);">
                    <td><span class="chip chip-danger" style="font-weight: 600;">3</span></td>
                    <td><strong style="color: var(--danger);">System Critical</strong></td>
                    <td><span class="chip chip-danger" style="font-weight: 600;">Deny (Enforced)</span></td>
                    <td class="text-muted">System binaries and host-critical config.</td>
                  </tr>
                  <tr>
                    <td><span class="chip chip-danger" style="font-weight: 600;">4</span></td>
                    <td style="font-weight: 500;">Secrets</td>
                    <td><span class="chip" style="background: var(--content-bg-alt);">${esc(dispLabel(fsRules.secrets))}</span></td>
                    <td class="text-muted">Credentials, keys, and sensitive material.</td>
                  </tr>
                </tbody>
              </table>
            </div>
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

    const tabs = document.querySelectorAll('[data-policy-tab]');
    tabs.forEach(tab => {
      tab.addEventListener('click', () => {
        tabs.forEach(t => t.classList.remove('active'));
        tab.classList.add('active');
        const target = tab.getAttribute('data-policy-tab');
        ['profile', 'network', 'domains', 'elevation', 'advanced', 'status'].forEach(id => {
           const el = document.getElementById('policy-tab-' + id);
           if (el) el.style.display = (id === target) ? '' : 'none';
        });
      });
    });


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


    // Single Global Save Action
    const saveBtn = document.getElementById('policy-save-all');
    const resetBtn = document.getElementById('policy-reset-all');

    const trackChanges = () => {
      if (!saveBtn || !resetBtn) return;
      saveBtn.disabled = false;
      resetBtn.style.display = 'inline-flex';
    };

    root.addEventListener('change', (e) => {
      const target = e.target;
      if (target.closest('#policy-main-tabs') || target.id === 'policy-profile') return;
      trackChanges();
    });
    
    root.addEventListener('input', (e) => {
      if (e.target.matches('input[type="text"], input[type="number"], input[type="search"]')) {
        trackChanges();
      }
    });

    root.addEventListener('click', (e) => {
      const target = e.target;
      const idsTriggers = [
        'policy-allowlist-add-btn', 'policy-denylist-add-btn',
        'policy-allow-package-add-btn', 'policy-deny-package-add-btn',
        'policy-reset-allowlist-defaults', 'policy-reset-denylist-defaults'
      ];
      if (target.closest('.domain-remove') || target.closest('.package-remove') || idsTriggers.includes(target.id)) {
        trackChanges();
      }
    });

    if (resetBtn) {
      resetBtn.addEventListener('click', () => {
        renderPolicy(root);
      });
    }

    if (saveBtn) {
      saveBtn.addEventListener('click', async () => {
        try {
          saveBtn.disabled = true;
          const origText = saveBtn.innerHTML;
          saveBtn.textContent = 'Saving...';

          const allowlistHosts = allowlistRows.filter(item => item.enabled).map(item => item.host);
          const denylistHosts = denylistRows.filter(item => item.enabled).map(item => item.host);
          
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

          const body = {
            network_default_deny: document.getElementById('toggle-network-deny').checked,
            network_require_approval_for_post: document.getElementById('toggle-network-post-approval').checked,
            network_allowlist_hosts: allowlistHosts,
            network_invert_allowlist: document.getElementById('toggle-invert-allowlist').checked,
            network_denylist_hosts: denylistHosts,
            network_invert_denylist: document.getElementById('toggle-invert-denylist').checked,
            
            elevation_enabled: document.getElementById('toggle-elevation-enabled').checked,
            elevation_require_operator_auth: document.getElementById('toggle-elevation-auth').checked,
            elevation_use_allowlist: document.getElementById('toggle-elevation-use-allowlist').checked,
            elevation_allowed_packages: collectEnabledPackages(allowPackageRows),
            elevation_denied_packages: collectEnabledPackages(denyPackageRows),
            
            fs_workspace: document.getElementById('policy-fs-workspace').value,
            fs_user_data: document.getElementById('policy-fs-user-data').value,
            fs_shared: document.getElementById('policy-fs-shared').value,
            fs_secrets: document.getElementById('policy-fs-secrets').value,
            deny_workspace_exec: document.getElementById('policy-exec-workspace').checked,
            deny_tmp_exec: document.getElementById('policy-exec-tmp').checked,
            quarantine_on_download_exec_chain: document.getElementById('policy-exec-quarantine').checked,
            deny_autostart: document.getElementById('policy-persistence-autostart').checked,
            mass_delete_threshold: Number(document.getElementById('policy-mass-delete-threshold').value) || 40
          };

          const updated = await api('/api/policy/toggles', {
            method: 'POST',
            body
          });
          state.policy = updated;
          toast('Configuration saved', 'success');
          await refreshStatus();
          await renderPolicy(root);
        } catch (err) {
          toast(`Failed to save policy: ${err.message}`, 'error');
          saveBtn.disabled = false;
          saveBtn.innerHTML = origText;
        }
      });
    }
  } // end renderPolicy

