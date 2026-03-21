  // ============================================
  // Initialization
  // ============================================

  function init() {
    try {
      selfCheckEscaping();
    } catch (err) {
      console.error(err);
      toast('UI escape check failed. Refresh after update.', 'error');
    }

    initThemeToggle();

    // Get current page from body data attribute
    state.currentPage = document.body.dataset.page || 'overview';
    
    // Bind navigation
    document.querySelectorAll('[data-page-link]').forEach(link => {
      if (link.dataset.pageLink === state.currentPage) {
        link.classList.add('active');
      }
    });

    bindSidebarCollapse();
    bindMobileSidebar();
    
    // Bind modal close
    const modalBackdrop = document.getElementById('modal-backdrop');
    if (modalBackdrop) {
      modalBackdrop.addEventListener('click', (e) => {
        if (e.target === modalBackdrop) closeModal();
      });
    }
    
    const modalClose = document.getElementById('modal-close');
    if (modalClose) {
      modalClose.addEventListener('click', closeModal);
    }
    
    // Request notification permission on first interaction
    document.addEventListener('click', () => {
      requestNotificationPermission();
    }, { once: true });
    
    // Initial load
    refreshStatus()
      .then(() => {
        renderPage();
        if (typeof window.preloadRunnersFleet === 'function') {
          window.preloadRunnersFleet();
        }
        return fetchUpdateStatus({ force: false, quiet: true });
      })
      .then(() => {
        // Initialize pending count without notifying
        lastPendingCount = state.status?.pending_approvals || 0;
      })
      .then(() => startPolling())
      .catch(err => {
        console.error('Initialization failed:', err);
        toast('Failed to initialize. Please refresh.', 'error');
      });
  }

  function startPolling() {
    stopPolling();
    state.pollTimer = setInterval(async () => {
      try {
        await refreshStatus();
        await fetchUpdateStatus({ force: false, quiet: true });
        const newCount = state.status?.pending_approvals || 0;
        
        // Check for new approvals and notify
        checkForNewApprovals(newCount);
        
        if (state.currentPage === 'approvals') {
          await loadApprovals();
        } else if (state.currentPage === 'approval-detail') {
          // Refresh approval detail if on that page
          const pathParts = window.location.pathname.split('/').filter(Boolean);
          const approvalId = decodeURIComponent(pathParts[pathParts.length - 1] || '');
          try {
            const data = await api(`/api/approvals/${approvalId}`);
            renderApprovalDetailContent(data);
          } catch (err) {
            // Ignore errors during background refresh
          }
        } else if (state.currentPage === 'receipts') {
          // Refresh receipts timeline if on that page
          try {
            if (state.receipts.mode === 'logs') {
              await loadUiLogs();
            } else {
              await loadReceipts();
            }
          } catch (err) {
            // Ignore errors during background refresh
            console.error('Receipts refresh error:', err);
          }
        }
      } catch (err) {
        console.error('Polling error:', err);
      }
    }, 15000); // Poll every 15 seconds
  }

  function stopPolling() {
    if (state.pollTimer) {
      clearInterval(state.pollTimer);
      state.pollTimer = null;
    }
  }

  // Expose global functions needed for inline handlers
  window.closeModal = closeModal;

  // Initialize on DOM ready
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
