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
            <h3>Run Doctor</h3>
            <p>Diagnose common runtime issues and optionally apply safe local repairs.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row" style="background: var(--content-bg); border: 1px solid var(--content-border); padding: var(--space-4); border-radius: var(--radius-lg);">
              <p class="form-hint" style="margin: 0 0 var(--space-3) 0;">Use <code class="mono">--repair</code> only when you want explicit safe local fixes.</p>
              <div style="display: flex; gap: var(--space-2); flex-wrap: wrap;">
                <button id="exec-doctor-run" class="btn btn-secondary" type="button">Run Doctor</button>
                <button id="exec-doctor-repair" class="btn btn-warning" type="button">Run Doctor (--repair)</button>
              </div>
            </div>
            <div id="exec-doctor-result" class="execution-output hidden"></div>
          </div>
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
            <div id="exec-one-shot-result" class="execution-output hidden"></div>
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

    document.getElementById('exec-doctor-run').addEventListener('click', () => runDoctor(false));
    document.getElementById('exec-doctor-repair').addEventListener('click', () => runDoctor(true));
    document.getElementById('reset-exec').addEventListener('click', resetExecution);
    document.getElementById('reset-runtime').addEventListener('click', resetRuntime);
    document.getElementById('exec-one-shot-run').addEventListener('click', runOneShotScript);
  }

  function executionStatusChip(label, value, tone) {
    const className = tone ? `execution-status-chip execution-status-chip-${tone}` : 'execution-status-chip';
    return `<span class="${className}"><strong>${esc(label)}:</strong> ${esc(String(value))}</span>`;
  }

  function executionRecommendationTone(kind) {
    if (kind === 'continue') return 'ok';
    if (kind === 'repair') return 'warn';
    return 'fail';
  }

  function renderExecutionOutput(resultEl, payload, options = {}) {
    const status = payload?.status || 'unknown';
    const exitCode = payload?.exit_code;
    const confinement = payload?.confinement || '-';
    const errorText = payload?.error || '';
    const stdout = payload?.stdout || '';
    const stderr = payload?.stderr || '';

    const normalizedStatus = String(status).toLowerCase();
    const statusTone = (normalizedStatus === 'completed' || normalizedStatus === 'ok')
      ? 'ok'
      : ((normalizedStatus === 'failed' || normalizedStatus === 'fail')
        ? 'fail'
        : 'warn');

    const primaryOutput = (options.primaryOutput || stdout || stderr || '(empty)');
    const primaryLabel = options.primaryLabel || (stdout ? 'stdout' : (stderr ? 'stderr' : 'output'));
    const showPrimary = options.showPrimary !== false;
    const copyText = typeof options.copyText === 'string' ? options.copyText : '';
    const copyButtonId = options.copyButtonId || '';
    const copyButtonLabel = options.copyButtonLabel || 'Copy';
    const chips = [
      executionStatusChip('Status', status, statusTone),
      Number.isFinite(exitCode) ? executionStatusChip('Exit', exitCode, exitCode === 0 ? 'ok' : 'fail') : '',
      executionStatusChip('Confinement', confinement, 'neutral')
    ].filter(Boolean).join('');
    const summaryText = options.summaryText || '';
    const recommendation = options.recommendation || null;
    const recommendationKind = recommendation?.kind || 'manual';
    const summaryHtml = summaryText ? `<div class="execution-output-summary">${esc(summaryText)}</div>` : '';
    const recommendationHtml = recommendation?.message ? `
      <div class="execution-output-recommendation execution-output-recommendation-${esc(executionRecommendationTone(recommendationKind))}">
        <strong>Recommended next step:</strong> ${esc(recommendation.message)}
      </div>
    ` : '';

    const showSecondaryStreams = !!options.showSecondaryStreams;
    const streamBlocks = [];
    if (showSecondaryStreams) {
      if (stdout) {
        streamBlocks.push(`
          <div class="execution-stream-block">
            <div class="execution-stream-label">stdout</div>
            <pre class="execution-stream-pre">${esc(stdout)}</pre>
          </div>
        `);
      }
      if (stderr) {
        streamBlocks.push(`
          <div class="execution-stream-block">
            <div class="execution-stream-label">stderr</div>
            <pre class="execution-stream-pre">${esc(stderr)}</pre>
          </div>
        `);
      }
      if (!streamBlocks.length) {
        streamBlocks.push(`
          <div class="execution-stream-block">
            <div class="execution-stream-label">output</div>
            <pre class="execution-stream-pre">(empty)</pre>
          </div>
        `);
      }
    }
    const secondary = showSecondaryStreams ? `
      <div class="execution-stream-grid">
        ${streamBlocks.join('')}
      </div>
    ` : '';
    const primary = showPrimary ? `
      <div class="execution-primary-block">
        <div class="execution-stream-label">${esc(primaryLabel)}</div>
        <pre class="execution-primary-pre">${esc(primaryOutput)}</pre>
      </div>
    ` : '';
    const actions = copyButtonId ? `
      <div class="execution-output-actions">
        <button id="${esc(copyButtonId)}" class="btn btn-secondary btn-sm" type="button">${esc(copyButtonLabel)}</button>
      </div>
    ` : '';

    resultEl.classList.remove('hidden');
    resultEl.innerHTML = `
      <div class="execution-output-card">
        <div class="execution-output-header">
          <div class="execution-output-header-row">
            <div class="execution-output-title">${esc(options.title || 'Execution Result')}</div>
            ${actions}
          </div>
          ${summaryHtml}
          ${options.showStatusChips === false ? '' : `<div class="execution-status-row">${chips}</div>`}
          ${recommendationHtml}
        </div>
        ${errorText ? `<div class="execution-output-error"><strong>Error:</strong> ${esc(errorText)}</div>` : ''}
        ${primary}
        ${secondary}
      </div>
    `;

    if (copyButtonId && copyText) {
      const copyBtn = document.getElementById(copyButtonId);
      if (copyBtn) {
        copyBtn.addEventListener('click', async () => {
          try {
            await copyTextToClipboard(copyText);
            toast('Copied output to clipboard', 'success');
          } catch (err) {
            toast(`Copy failed: ${err.message || err}`, 'error');
          }
        });
      }
    }
  }

  async function copyTextToClipboard(text) {
    if (navigator?.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return;
    }
    const fallback = document.createElement('textarea');
    fallback.value = text;
    fallback.setAttribute('readonly', '');
    fallback.style.position = 'fixed';
    fallback.style.opacity = '0';
    document.body.appendChild(fallback);
    fallback.select();
    const ok = document.execCommand('copy');
    document.body.removeChild(fallback);
    if (!ok) {
      throw new Error('clipboard unavailable');
    }
  }

  async function runDoctor(repair) {
    const runBtn = document.getElementById('exec-doctor-run');
    const repairBtn = document.getElementById('exec-doctor-repair');
    const resultEl = document.getElementById('exec-doctor-result');
    if (!resultEl) return;

    if (runBtn) runBtn.disabled = true;
    if (repairBtn) repairBtn.disabled = true;
    resultEl.classList.remove('hidden');
    resultEl.innerHTML = '<div class="text-muted">Running doctor checks...</div>';

    try {
      const res = await fetch('/api/doctor', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ repair: !!repair })
      });
      const payload = await res.json().catch(() => ({}));
      const status = payload.status || (res.ok ? 'ok' : 'fail');
      const doctorStatus = String(status).toLowerCase();
      const doctorExitCode = doctorStatus === 'fail' ? 1 : 0;

      renderExecutionOutput(resultEl, {
        status,
        exit_code: doctorExitCode,
        confinement: 'n/a',
        error: res.ok ? '' : (payload.error || 'doctor execution failed'),
        stdout: payload.output || '',
        stderr: payload.summary_line || ''
      }, {
        title: repair ? 'Doctor Result (--repair)' : 'Doctor Result',
        primaryLabel: 'doctor output',
        primaryOutput: payload.output || '(no doctor output)',
        copyButtonId: 'exec-doctor-copy-output',
        copyButtonLabel: 'Copy Result',
        copyText: payload.output || '',
        showSecondaryStreams: false,
        showStatusChips: false,
        summaryText: payload.summary_line || '',
        recommendation: payload.recommendation || null
      });

      if (res.ok) {
        toast(repair ? 'Doctor completed with repair mode' : 'Doctor completed', 'success');
      } else {
        toast('Doctor finished with errors', 'warning');
      }
    } catch (err) {
      resultEl.classList.remove('hidden');
      resultEl.innerHTML = `<div class="text-danger">Doctor failed: ${esc(err.message)}</div>`;
      toast(`Doctor failed: ${err.message}`, 'error');
    } finally {
      if (runBtn) runBtn.disabled = false;
      if (repairBtn) repairBtn.disabled = false;
    }
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
    if (!resultEl) return;
    if (runBtn) runBtn.disabled = true;

    resultEl.classList.remove('hidden');
    resultEl.innerHTML = '<div class="text-muted">Running command...</div>';

    try {
      const res = await fetch('/api/run/script', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ script })
      });
      const payload = await res.json().catch(() => ({}));

      renderExecutionOutput(resultEl, payload, {
        title: 'One-Shot Result',
        showPrimary: false,
        showSecondaryStreams: true,
        summaryText: payload.summary_line || ''
      });

      if (res.ok) {
        toast('One-shot command completed', 'success');
      } else {
        toast('One-shot command finished with errors', 'warning');
      }
      await refreshStatus();
    } catch (err) {
      resultEl.classList.remove('hidden');
      resultEl.innerHTML = `<div class="text-danger">Run failed: ${esc(err.message)}</div>`;
      toast(`Run failed: ${err.message}`, 'error');
    } finally {
      if (runBtn) runBtn.disabled = false;
    }
  }
