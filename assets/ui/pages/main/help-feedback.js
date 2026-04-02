  // ============================================
  // Help / Feedback Page
  // ============================================

  function renderHelpFeedback(root) {
    const issuesUrl = 'https://github.com/steadeepanda/agent-ruler/issues/new?template=bug_report.yml';
    const discussionsUrl = 'https://github.com/steadeepanda/agent-ruler/discussions';

    root.innerHTML = `
      <div class="settings-container">
        <div class="settings-header">
          <h2 class="settings-title">Help / Feedback</h2>
          <p class="settings-description">Report bugs, share ideas, and get support for Agent Ruler.</p>
        </div>

        <div class="settings-section">
          <div class="settings-section-header">
            <h3>Report a bug</h3>
            <p>Use the issue form so maintainers get the diagnostics required to reproduce quickly.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row" style="background: var(--content-bg); border: 1px solid var(--content-border); padding: var(--space-4); border-radius: var(--radius-lg);">
              <p style="margin: 0 0 var(--space-3) 0; color: var(--text-secondary);">Include your steps, logs, and <code class="mono">agent-ruler doctor</code> output when possible.</p>
              <div>
                <a href="${issuesUrl}" target="_blank" rel="noopener" class="btn btn-primary">Open Bug Report Form</a>
              </div>
            </div>
          </div>
        </div>

        <div class="settings-section">
          <div class="settings-section-header">
            <h3>Feedback / Ideas</h3>
            <p>Share product ideas and feature requests in GitHub Discussions.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row" style="background: var(--content-bg); border: 1px solid var(--content-border); padding: var(--space-4); border-radius: var(--radius-lg);">
              <p style="margin: 0 0 var(--space-3) 0; color: var(--text-secondary);">Discussion threads help group proposals and track decisions.</p>
              <div>
                <a href="${discussionsUrl}" target="_blank" rel="noopener" class="btn btn-secondary">Open Discussions</a>
              </div>
            </div>
          </div>
        </div>

        <div class="settings-section" style="border-bottom: none;">
          <div class="settings-section-header">
            <h3>Ask a question / get help</h3>
            <p>Start here before opening a bug.</p>
          </div>
          <div class="settings-section-content">
            <div class="settings-row" style="background: var(--content-bg); border: 1px solid var(--content-border); padding: var(--space-4); border-radius: var(--radius-lg);">
              <p style="margin: 0 0 var(--space-2) 0;"><strong>Quick checklist:</strong></p>
              <p style="margin: 0 0 var(--space-1) 0; color: var(--text-secondary);">1. Read the <a href="/help/" target="_blank" rel="noopener">Documentation</a>.</p>
              <p style="margin: 0 0 var(--space-1) 0; color: var(--text-secondary);">2. Run <code class="mono">agent-ruler doctor</code>.</p>
              <p style="margin: 0 0 var(--space-3) 0; color: var(--text-secondary);">3. Ask in GitHub Discussions if you still need help.</p>
              <div style="display: flex; gap: var(--space-2); flex-wrap: wrap;">
                <a href="/help/" target="_blank" rel="noopener" class="btn btn-secondary">Open Documentation</a>
                <a href="${discussionsUrl}" target="_blank" rel="noopener" class="btn btn-secondary">Ask in Discussions</a>
              </div>
            </div>
          </div>
        </div>
      </div>
    `;
  }
