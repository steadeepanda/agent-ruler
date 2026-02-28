  // Extracted from assets/ui/20-pages-config.js for configuration-scoped editing.
  // ============================================
  // Execution Page
  // ============================================

  function renderExecution(root) {
    root.innerHTML = `
      <div class="grid grid-2">
        <div class="card">
          <div class="card-header">
            <h3 class="card-title">One-Shot Command Runner</h3>
          </div>
          <div class="card-body">
            <p class="text-muted mb-4">
              Run a single troubleshooting command inside Agent Ruler confinement and inspect deterministic results.
            </p>
            <div class="form-group">
              <label class="form-label">Command (bash)</label>
              <textarea id="exec-one-shot-script" class="form-textarea" placeholder="example: echo hello && ls -la"></textarea>
              <p class="form-hint">Runs through <code>/api/run/script</code> and writes receipts visible in Timeline.</p>
            </div>
            <button id="exec-one-shot-run" class="btn btn-primary">Run Command</button>
            <div id="exec-one-shot-result" class="diff-preview hidden mt-4"></div>
          </div>
        </div>

        <div class="card">
          <div class="card-header">
            <h3 class="card-title">Execution Layer</h3>
          </div>
          <div class="card-body">
            <p class="text-muted mb-4">
              The execution layer contains ephemeral state for running agent processes.
              Resetting it will clear this state but preserve persistent data like receipts and approvals.
            </p>
            <p class="form-hint mb-4">
              Operator note: run-command endpoints are for deterministic troubleshooting; routine agent work should run through normal agent tasks and policy gates.
            </p>
            
            <div class="alert alert-warning mb-4">
              <span class="alert-icon">⚠️</span>
              <div class="alert-content">
                <div class="alert-title">Warning</div>
                <div class="alert-message">Resetting the execution layer will terminate any running agent processes.</div>
              </div>
            </div>
          </div>
          <div class="card-footer">
            <button id="reset-exec" class="btn btn-danger">Reset Execution Layer</button>
          </div>
        </div>
      </div>

      <div class="card mt-5">
        <div class="card-header">
          <h3 class="card-title">Reset Agent Ruler Runtime</h3>
        </div>
        <div class="card-body">
          <p class="text-muted mb-4">
            Use this when runtime state gets messy and you want a clean baseline without reinstalling.
          </p>
          <label class="form-check mb-3">
            <input type="checkbox" id="reset-runtime-keep-config" class="form-check-input" checked />
            <span class="form-check-label">Keep current config and policy</span>
          </label>
          <p class="form-hint mb-4">
            If enabled: keeps your config/policy wiring (agent ↔ ruler ↔ user paths and toggles), while clearing runtime state artifacts.
            If disabled: restores default config/policy plus fresh runtime state.
          </p>
          <div class="alert alert-warning">
            <span class="alert-icon">⚠️</span>
            <div class="alert-content">
              <div class="alert-title">Reset Scope</div>
              <div class="alert-message">Runtime reset removes receipts, approvals, staged exports, and execution artifacts under the runtime root.</div>
            </div>
          </div>
        </div>
        <div class="card-footer">
          <button id="reset-runtime" class="btn btn-danger">Reset Runtime</button>
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
