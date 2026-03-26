  // Extracted from assets/ui/20-pages-config.js for configuration-scoped editing.
  // ============================================
  // Execution Page
  // ============================================

  function renderExecution(root) {
    root.innerHTML = `
      <div class="settings-container">
        <div class="settings-header">
          <h2 class="settings-title">Execution Layer</h2>
          <p class="settings-description">Perform one-shot commands, troubleshoot, and manage runner state.</p>
        </div>

        <div class="settings-section">
          <div class="settings-section-header">
            <h3>One-Shot Command</h3>
            <p>Run a single script inside Agent Ruler confinement and inspect deterministic results.</p>
            <p class="form-hint" style="margin-top: var(--space-4);">Operator note: run-command endpoints are for deterministic troubleshooting; routine agent work should run through normal agent tasks and policy gates.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row" style="background: var(--content-bg); border: 1px solid var(--content-border); padding: var(--space-4); border-radius: var(--radius-lg);">
              <label class="form-label">Command Script (bash)</label>
              <textarea id="exec-one-shot-script" class="form-textarea mono" placeholder="example: echo hello && ls -la" style="min-height: 120px; font-size: 0.85rem;"></textarea>
              <p class="form-hint" style="margin-top: var(--space-2);">Runs through <code class="mono">/api/run/script</code> and writes receipts visible in Timeline.</p>
              <div class="mt-4">
                <button id="exec-one-shot-run" class="btn btn-secondary" style="align-self: flex-start;">Run Command</button>
              </div>
            </div>
            <div id="exec-one-shot-result" class="diff-preview hidden" style="border-radius: var(--radius-lg); overflow: hidden; margin-top: var(--space-2);"></div>
          </div>
        </div>

        <div class="settings-section">
          <div class="settings-section-header">
            <h3>Reset Execution Layer</h3>
            <p>Terminate running processes and clear ephemeral state.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row" style="background: var(--danger-bg); border: 1px solid var(--danger-border); padding: var(--space-4); border-radius: var(--radius-lg);">
              <div style="display: flex; gap: var(--space-3); color: var(--danger);">
                <span style="font-size: 1.2rem;">⚠️</span>
                <div>
                  <h4 style="font-weight: 600; font-size: 0.95rem; margin: 0 0 var(--space-1) 0; color: var(--danger);">Reset Execution Layer Mode</h4>
                  <p style="font-size: 0.85rem; margin: 0; color: var(--text-primary);">This will terminate any running agent processes immediately. Receipts and approvals are preserved.</p>
                </div>
              </div>
              <div class="mt-4">
                <button id="reset-exec" class="btn btn-danger" style="align-self: flex-start;">Reset Execution Layer</button>
              </div>
            </div>
          </div>
        </div>

        <div class="settings-section" style="border-bottom: none;">
          <div class="settings-section-header">
            <h3>Reset Agent Ruler Runtime</h3>
            <p>Use this when runtime state gets messy and you want a clean baseline without reinstalling.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row" style="background: var(--bg-primary); border: 1px solid var(--danger-border); padding: var(--space-4); border-radius: var(--radius-lg);">
              <label class="form-switch" style="align-items: center;">
                <input type="checkbox" id="reset-runtime-keep-config" class="form-switch-input" checked />
                <div class="form-switch-text">
                  <span class="form-switch-label" style="font-weight: 600;">Keep current config and policy</span>
                </div>
              </label>
              <div style="font-size: 0.85rem; color: var(--text-primary); margin-top: var(--space-4); line-height: 1.5; padding-left: 2px;">
                <p style="margin: 0 0 var(--space-2) 0;"><strong>If checked:</strong> keeps your config/policy wiring (agent ↔ ruler ↔ user paths and toggles), while clearing runtime state artifacts.</p>
                <p style="margin: 0 0 var(--space-2) 0;"><strong>If unchecked:</strong> restores default config/policy and creates fresh runtime state.</p>
                <p style="margin: 0; color: var(--danger); font-weight: 500;">Note: Runtime reset removes receipts, approvals, staged exports, and execution artifacts under the runtime root.</p>
              </div>
              <div class="mt-4">
                <button id="reset-runtime" class="btn btn-danger" style="align-self: flex-start;">Reset Runtime</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    `;
    
    document.getElementById('reset-exec').addEventListener('click', resetExecution);
    document.getElementById('reset-runtime').addEventListener('click', resetRuntime);
    document.getElementById('exec-one-shot-run').addEventListener('click', runOneShotScript);
  }

  async function resetExecution() {
    if (!confirm('Are you sure you want to reset the execution layer? This will terminate running processes.')) return;
    
    try {
      await api('/api/reset-exec', { method: 'POST' });
      toast('Execution layer reset successfully', 'success');
    } catch (err) {
      toast(`Failed to reset: ${err.message}`, 'error');
    }
  }

  async function resetRuntime() {
    const keepConfig = !!document.getElementById('reset-runtime-keep-config')?.checked;
    const confirmMsg = keepConfig
      ? 'Reset runtime while keeping current config and policy?'
      : 'Reset runtime and restore default config/policy?';
    if (!confirm(confirmMsg)) return;

    try {
      const result = await api('/api/reset-runtime', {
        method: 'POST',
        body: { keep_config: keepConfig }
      });
      toast(result?.message || 'Runtime reset completed', 'success');
      await refreshStatus();
      renderPage();
    } catch (err) {
      toast(`Failed to reset runtime: ${err.message}`, 'error');
    }
  }

  async function runOneShotScript() {
    const script = (document.getElementById('exec-one-shot-script')?.value || '').trim();
    if (!script) {
      toast('Enter a command to run', 'warning');
      return;
    }

    const runBtn = document.getElementById('exec-one-shot-run');
    const resultEl = document.getElementById('exec-one-shot-result');
    if (runBtn) runBtn.disabled = true;
    if (resultEl) {
      resultEl.classList.remove('hidden');
      resultEl.innerHTML = '<div class="text-muted">Running command...</div>';
    }

    try {
      const res = await fetch('/api/run/script', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ script })
      });
      const payload = await res.json().catch(() => ({}));

      const status = payload.status || (res.ok ? 'completed' : 'failed');
      const exitCode = payload.exit_code;
      const confinement = payload.confinement || '-';
      const errorText = payload.error || '';
      const stdout = payload.stdout || '';
      const stderr = payload.stderr || '';

      if (resultEl) {
        resultEl.classList.remove('hidden');
        resultEl.innerHTML = `
          <div class="diff-summary">
            <strong>Status:</strong> ${esc(status)}
            ${Number.isFinite(exitCode) ? ` | <strong>Exit:</strong> ${esc(String(exitCode))}` : ''}
            | <strong>Confinement:</strong> ${esc(confinement)}
          </div>
          ${errorText ? `<div class="text-danger mt-2"><strong>Error:</strong> ${esc(errorText)}</div>` : ''}
          <div class="mt-3">
            <strong>stdout</strong>
            <pre class="code-block">${esc(stdout || '(empty)')}</pre>
          </div>
          <div class="mt-3">
            <strong>stderr</strong>
            <pre class="code-block">${esc(stderr || '(empty)')}</pre>
          </div>
        `;
      }

      if (res.ok) {
        toast('One-shot command completed', 'success');
      } else {
        toast('One-shot command finished with errors', 'warning');
      }
      await refreshStatus();
    } catch (err) {
      if (resultEl) {
        resultEl.classList.remove('hidden');
        resultEl.innerHTML = `<div class="text-danger">Run failed: ${esc(err.message)}</div>`;
      }
      toast(`Run failed: ${err.message}`, 'error');
    } finally {
      if (runBtn) runBtn.disabled = false;
    }
  }
