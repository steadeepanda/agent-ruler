/**
 * Agent Ruler - Modern Admin Console UI
 * 
 * A clean, professional admin interface for managing agent security.
 * Implements all CLI workflows through an intuitive web interface.
 */

(function() {
  'use strict';

  const THEME_STORAGE_KEY = 'vitepress-theme-appearance';
  const RECEIPT_DETAILS_STORAGE_KEY = 'ar.receipts.show_details';
  const PATH_DISPLAY_RUNTIME_ALIAS_STORAGE_KEY = 'ar.paths.use_runtime_aliases';
  const RECEIPT_RUNTIME_ALIAS_STORAGE_KEY = 'ar.receipts.use_runtime_aliases';
  const TIMELINE_MODE_STORAGE_KEY = 'ar.timeline.mode';
  const RUNNER_FILTER_STORAGE_KEY = 'ar.runner.filter';

  function readRuntimeAliasPreference() {
    const current = localStorage.getItem(PATH_DISPLAY_RUNTIME_ALIAS_STORAGE_KEY);
    if (current === '0') return false;
    if (current === '1') return true;
    const legacy = localStorage.getItem(RECEIPT_RUNTIME_ALIAS_STORAGE_KEY);
    return legacy !== '0';
  }

  // ============================================
  // State Management
  // ============================================
  
  const state = {
    currentPage: '',
    status: null,
    runtime: null,
    update: null,
    config: null,
    policy: null,
    profiles: [],
    domainPresets: null,
    approvals: [],
    runnerFilter: (function() {
      const stored = localStorage.getItem(RUNNER_FILTER_STORAGE_KEY) || 'all';
      return ['all', 'openclaw', 'claudecode', 'opencode'].includes(stored) ? stored : 'all';
    })(),
    pathDisplay: {
      useRuntimeAliases: readRuntimeAliasPreference()
    },
    receipts: {
      items: [],
      total: 0,
      limit: 50,
      offset: 0,
      hasMore: false,
      mode: localStorage.getItem(TIMELINE_MODE_STORAGE_KEY) === 'logs' ? 'logs' : 'receipts',
      showDetails: localStorage.getItem(RECEIPT_DETAILS_STORAGE_KEY) === '1',
      filters: {
        date: '',
        q: '',
        verdict: '',
        action: '',
      }
    },
    logs: {
      items: [],
      total: 0,
      limit: 50,
      offset: 0,
      hasMore: false,
      filters: {
        level: '',
        source: '',
        q: '',
      }
    },
    stagedExports: [],
    pollTimer: null,
    isLoading: false
  };

  // ============================================
  // Utilities
  // ============================================

  function esc(str) {
    return String(str || '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function selfCheckEscaping() {
    const probe = `&<>"'`;
    const expected = '&amp;&lt;&gt;&quot;&#39;';
    if (esc(probe) !== expected) {
      throw new Error('Escaping self-check failed');
    }
  }
  function formatBytes(bytes) {
    const n = Number(bytes || 0);
    if (!Number.isFinite(n) || n <= 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB'];
    let val = n;
    let i = 0;
    while (val >= 1024 && i < units.length - 1) {
      val /= 1024;
      i++;
    }
    return `${val.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function formatDiff(summary) {
    if (!summary) return 'No changes';
    const parts = [];
    if (summary.files_added) parts.push(`<span class="text-success">+${summary.files_added} files</span>`);
    if (summary.files_removed) parts.push(`<span class="text-danger">-${summary.files_removed} files</span>`);
    if (summary.files_changed) parts.push(`<span class="text-warning">~${summary.files_changed} files</span>`);
    if (summary.bytes_added) parts.push(`<span class="text-success">+${formatBytes(summary.bytes_added)}</span>`);
    if (summary.bytes_removed) parts.push(`<span class="text-danger">-${formatBytes(summary.bytes_removed)}</span>`);
    return parts.length ? parts.join(' ') : 'No changes';
  }

  function formatTimestamp(ts) {
    if (!ts) return '-';
    const date = new Date(ts);
    return date.toLocaleString('en-US', {
      month: '2-digit',
      day: '2-digit',
      year: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hour12: false
    });
  }

  function formatRelativeTime(ts) {
    if (!ts) return '-';
    const date = new Date(ts);
    const now = new Date();
    const diffMs = now - date;
    const diffMins = Math.floor(diffMs / 60000);
    const diffHours = Math.floor(diffMs / 3600000);
    const diffDays = Math.floor(diffMs / 86400000);
    
    if (diffMins < 1) return 'Just now';
    if (diffMins < 60) return `${diffMins}m ago`;
    if (diffHours < 24) return `${diffHours}h ago`;
    if (diffDays < 7) return `${diffDays}d ago`;
    return formatTimestamp(ts);
  }

  function formatTimelineTimestamp(ts, activeDateFilter) {
    if (!ts) return '-';
    const date = new Date(ts);
    const now = new Date();
    const sameDay = date.getFullYear() === now.getFullYear()
      && date.getMonth() === now.getMonth()
      && date.getDate() === now.getDate();

    const time = date.toLocaleTimeString('en-US', {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hour12: false
    });
    if (!activeDateFilter && sameDay) {
      return `Today ${time}`;
    }
    return formatTimestamp(ts);
  }

  function verdictClass(verdict) {
    const v = String(verdict || '').toLowerCase();
    if (v.includes('require') || v.includes('approval')) return 'require_approval';
    if (v.includes('deny')) return 'deny';
    if (v.includes('quarantine')) return 'quarantine';
    return 'allow';
  }

  function verdictIcon(verdict) {
    const v = String(verdict || '').toLowerCase();
    if (v.includes('allow')) return '✓';
    if (v.includes('deny')) return '✕';
    if (v.includes('require') || v.includes('approval')) return '?';
    if (v.includes('quarantine')) return '!';
    return '•';
  }

  function getFlowSource() {
    return localStorage.getItem('ar.flow.source') || 'user';
  }

  function setFlowSource(value) {
    localStorage.setItem('ar.flow.source', value);
  }

  const DRAG_DROP_AUTO_APPROVE_KEY = 'ar.flow.drag_drop.auto_approve';

  function getDragDropAutoApprovePreference() {
    const value = localStorage.getItem(DRAG_DROP_AUTO_APPROVE_KEY);
    if (value === '0') return false;
    if (value === '1') return true;
    return true;
  }

  function setDragDropAutoApprovePreference(enabled) {
    localStorage.setItem(DRAG_DROP_AUTO_APPROVE_KEY, enabled ? '1' : '0');
  }

  function getPreviewMode() {
    return localStorage.getItem('ar.flow.preview') || 'auto';
  }

  function setPreviewMode(value) {
    localStorage.setItem('ar.flow.preview', value);
  }

  function setReceiptDetailVisibility(enabled) {
    const value = !!enabled;
    state.receipts.showDetails = value;
    localStorage.setItem(RECEIPT_DETAILS_STORAGE_KEY, value ? '1' : '0');
  }

  function setRuntimeAliasVisibility(enabled) {
    const value = !!enabled;
    state.pathDisplay.useRuntimeAliases = value;
    localStorage.setItem(PATH_DISPLAY_RUNTIME_ALIAS_STORAGE_KEY, value ? '1' : '0');
    // Keep legacy key in sync for backward compatibility with older UI bundles.
    localStorage.setItem(RECEIPT_RUNTIME_ALIAS_STORAGE_KEY, value ? '1' : '0');
  }

  function setTimelineMode(mode) {
    const normalized = mode === 'logs' ? 'logs' : 'receipts';
    state.receipts.mode = normalized;
    localStorage.setItem(TIMELINE_MODE_STORAGE_KEY, normalized);
  }

  function setRunnerFilter(runnerId) {
    const normalized = normalizeRunnerFilter(runnerId);
    state.runnerFilter = normalized;
    localStorage.setItem(RUNNER_FILTER_STORAGE_KEY, normalized);
  }

  function normalizeRunnerFilter(value) {
    const normalized = String(value || '').trim().toLowerCase();
    if (normalized === 'openclaw' || normalized === 'claudecode' || normalized === 'opencode') {
      return normalized;
    }
    return 'all';
  }

  function runnerFilterOptions() {
    return [
      { id: 'all', label: 'All' },
      { id: 'openclaw', label: 'OpenClaw' },
      { id: 'claudecode', label: 'Claude Code' },
      { id: 'opencode', label: 'OpenCode' }
    ];
  }

  function runtimePathMappings() {
    const runtime = state.runtime || {};
    const mappings = [
      ['WORKSPACE_PATH', runtime.workspace],
      ['SHARED_ZONE_PATH', runtime.shared_zone],
      ['STATE_PATH', runtime.state_dir],
      ['RUNTIME_ROOT', runtime.runtime_root],
      ['DELIVERY_PATH', runtime.default_user_destination_dir || runtime.default_delivery_dir],
      ['RULER_ROOT', runtime.ruler_root]
    ].filter((entry) => !!entry[1]);
    mappings.sort((a, b) => String(b[1]).length - String(a[1]).length);
    return mappings;
  }

  function aliasRuntimePath(rawPath) {
    const value = String(rawPath || '').trim();
    if (!value || !state.pathDisplay.useRuntimeAliases) return value;
    const mappings = runtimePathMappings();

    for (const [label, prefix] of mappings) {
      const normalizedPrefix = String(prefix || '').replace(/\/+$/, '');
      if (!normalizedPrefix) continue;
      if (value === normalizedPrefix) return label;
      if (value.startsWith(`${normalizedPrefix}/`)) {
        return `${label}${value.slice(normalizedPrefix.length)}`;
      }
    }
    return value;
  }

  function aliasRuntimeText(rawText) {
    const value = String(rawText || '');
    if (!value || !state.pathDisplay.useRuntimeAliases) return value;
    let output = value;
    for (const [label, prefix] of runtimePathMappings()) {
      const normalizedPrefix = String(prefix || '').replace(/\/+$/, '');
      if (!normalizedPrefix) continue;
      output = output.split(normalizedPrefix).join(label);
    }
    return output;
  }

  function readThemePreference() {
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    return stored === 'light' ? 'light' : 'dark';
  }

  function applyTheme(theme) {
    const normalized = theme === 'light' ? 'light' : 'dark';
    document.documentElement.dataset.theme = normalized;
    document.documentElement.classList.toggle('dark', normalized === 'dark');
    localStorage.setItem(THEME_STORAGE_KEY, normalized);
    updateThemeToggleButton(normalized);
  }

  function updateThemeToggleButton(theme) {
    const button = document.getElementById('theme-toggle');
    if (!button) return;

    const isDark = theme === 'dark';
    button.setAttribute('title', isDark ? 'Switch to Light Mode' : 'Switch to Dark Mode');
    button.setAttribute('aria-pressed', isDark ? 'true' : 'false');
  }

  function initThemeToggle() {
    applyTheme(readThemePreference());
    const button = document.getElementById('theme-toggle');
    if (!button || button.dataset.bound === '1') return;
    button.dataset.bound = '1';
    button.addEventListener('click', () => {
      applyTheme(readThemePreference() === 'dark' ? 'light' : 'dark');
    });
  }

  // ============================================
  // API Layer
  // ============================================

  async function api(url, options = {}) {
    const config = {
      ...options,
      headers: {
        'Content-Type': 'application/json',
        ...options.headers
      }
    };
    
    if (options.body && typeof options.body === 'object') {
      config.body = JSON.stringify(options.body);
    }
    
    const res = await fetch(url, config);
    const data = await res.json().catch(() => ({}));
    
    if (!res.ok) {
      const error = new Error(data.error || data.detail || `HTTP ${res.status}`);
      error.status = res.status;
      error.data = data;
      throw error;
    }
    
    return data;
  }

  let lastUiEventSignature = '';
  let lastUiEventAtMs = 0;

  async function recordUiEvent(level, source, message, details = null) {
    const safeLevel = String(level || '').trim().toLowerCase();
    const safeSource = String(source || '').trim();
    const safeMessage = String(message || '').trim();
    if (!safeLevel || !safeSource || !safeMessage) return;

    const signature = `${safeLevel}|${safeSource}|${safeMessage}`;
    const now = Date.now();
    if (signature === lastUiEventSignature && (now - lastUiEventAtMs) < 5000) return;
    lastUiEventSignature = signature;
    lastUiEventAtMs = now;

    try {
      await fetch('/api/ui/logs/event', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json'
        },
        body: JSON.stringify({
          level: safeLevel,
          source: safeSource,
          message: safeMessage.slice(0, 2000),
          details: details && typeof details === 'object' ? details : null
        })
      });
    } catch (_) {
      // Best-effort only.
    }
  }

  async function uploadImportFile(file) {
    const form = new FormData();
    form.append('file', file);

    const res = await fetch('/api/import/upload', {
      method: 'POST',
      body: form
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) {
      throw new Error(data.error || data.detail || `HTTP ${res.status}`);
    }
    return data;
  }

  // ============================================
  // Toast Notifications
  // ============================================

  function toast(message, type = 'info', duration = 4000, options = {}) {
    const container = document.getElementById('toast-container');
    if (!container) return;
    
    const icons = {
      success: '✓',
      error: '✕',
      warning: '!',
      info: 'ℹ'
    };
    
    const el = document.createElement('div');
    el.className = `toast ${type}`;
    const actionLink =
      options && options.linkHref
        ? `<a class="toast-link" href="${esc(options.linkHref)}">${esc(options.linkLabel || 'Open')}</a>`
        : '';
    el.innerHTML = `
      <span class="toast-icon">${icons[type]}</span>
      <div class="toast-content">
        <div class="toast-message">${esc(message)}</div>
        ${actionLink ? `<div class="toast-action">${actionLink}</div>` : ''}
      </div>
    `;
    
    container.appendChild(el);
    
    setTimeout(() => {
      el.style.opacity = '0';
      setTimeout(() => el.remove(), 300);
    }, duration);
  }

  // ============================================
  // Modal Management
  // ============================================

  function openModal(title, content, options = {}) {
    const backdrop = document.getElementById('modal-backdrop');
    const modalTitle = document.getElementById('modal-title');
    const modalBody = document.getElementById('modal-body');
    const modalFooter = document.getElementById('modal-footer');
    
    if (!backdrop) return;
    
    modalTitle.textContent = title;
    modalBody.innerHTML = content;
    modalFooter.innerHTML = options.footer || '';
    
    backdrop.classList.add('active');
    document.body.style.overflow = 'hidden';
    
    // Focus first input
    const firstInput = modalBody.querySelector('input, select, textarea');
    if (firstInput) setTimeout(() => firstInput.focus(), 100);
  }

  function closeModal() {
    const backdrop = document.getElementById('modal-backdrop');
    if (!backdrop) return;
    
    backdrop.classList.remove('active');
    document.body.style.overflow = '';
  }

  // ============================================
  // Page Rendering
  // ============================================

  function renderPage() {
    const page = state.currentPage;
    const root = document.getElementById('page-root');
    if (!root) return;
    
    switch (page) {
      case 'overview':
        renderOverview(root);
        break;
      case 'approvals':
        renderApprovals(root);
        break;
      case 'approval-detail':
        renderApprovalDetail(root);
        break;
      case 'files':
        renderFiles(root);
        break;
      case 'policy':
        renderPolicy(root);
        break;
      case 'receipts':
        renderReceipts(root);
        break;
      case 'runners':
        renderRunners(root);
        break;
      case 'runtime':
        renderRuntime(root);
        break;
      case 'execution':
        renderExecution(root);
        break;
      case 'settings':
        renderSettings(root);
        break;
      default:
        renderOverview(root);
    }
    
    updateSidebarInfo();
  }
