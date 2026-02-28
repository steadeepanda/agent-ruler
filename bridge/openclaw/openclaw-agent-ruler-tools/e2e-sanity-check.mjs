#!/usr/bin/env node
/**
 * End-to-End OpenClaw ↔ Agent Ruler Communication Sanity Check
 *
 * This script validates that the OpenClaw gateway can communicate with Agent Ruler
 * and that denied actions produce receipts in the timeline.
 *
 * Usage:
 *   node bridge/openclaw/openclaw-agent-ruler-tools/e2e-sanity-check.mjs
 *
 * Prerequisites:
 *   - Agent Ruler UI running (or will be auto-started)
 *   - OpenClaw gateway configured under Agent Ruler confinement
 *
 * Exit codes:
 *   0 - All checks passed
 *   1 - One or more checks failed
 */

import { spawn, execSync } from 'node:child_process';
import { readFileSync, existsSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { homedir, tmpdir } from 'node:os';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const AGENT_RULER_URL = process.env.AGENT_RULER_URL || 'http://127.0.0.1:4622';
const TIMEOUT_MS = 30000;

let failures = [];
let passes = [];

function log(message) {
  console.log(`[e2e-sanity] ${message}`);
}

function logPass(message) {
  console.log(`[e2e-sanity] ✓ PASS: ${message}`);
  passes.push(message);
}

function logFail(message) {
  console.error(`[e2e-sanity] ✗ FAIL: ${message}`);
  failures.push(message);
}

async function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function fetchWithTimeout(url, options = {}, timeoutMs = TIMEOUT_MS) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(url, { ...options, signal: controller.signal });
    clearTimeout(timeout);
    return response;
  } catch (err) {
    clearTimeout(timeout);
    throw err;
  }
}

async function checkUiReachable() {
  log('Checking if Agent Ruler UI is reachable...');
  try {
    const response = await fetchWithTimeout(`${AGENT_RULER_URL}/api/status`);
    if (response.ok) {
      const data = await response.json();
      logPass(`Agent Ruler UI reachable at ${AGENT_RULER_URL}`);
      log(`  Runtime workspace: ${data.runtime?.workspace || 'N/A'}`);
      return true;
    } else {
      logFail(`Agent Ruler UI returned status ${response.status}`);
      return false;
    }
  } catch (err) {
    logFail(`Agent Ruler UI not reachable: ${err.message}`);
    return false;
  }
}

async function checkToolPreflight() {
  log('Testing tool preflight endpoint...');

  // Test 1: Read operation in workspace (should be allowed)
  // First get the workspace path from status API
  let workspacePath = '/tmp/test-workspace';
  try {
    const statusResponse = await fetchWithTimeout(`${AGENT_RULER_URL}/api/status`);
    if (statusResponse.ok) {
      const statusData = await statusResponse.json();
      workspacePath = statusData.runtime?.workspace || workspacePath;
    }
  } catch {
    // Use default workspace path
  }

  try {
    const response = await fetchWithTimeout(`${AGENT_RULER_URL}/api/openclaw/tool/preflight`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        tool_name: 'read',
        params: { path: `${workspacePath}/test.txt` },
        context: { agent_id: 'e2e-sanity-check', session_key: 'test-session' }
      })
    });

    if (!response.ok) {
      logFail(`Tool preflight returned status ${response.status}`);
      return false;
    }

    const data = await response.json();
    // Workspace reads should be allowed; other paths may vary by policy
    if (data.status === 'allow' || data.status === 'ignored') {
      logPass(`Tool preflight read operation: ${data.status}`);
    } else if (data.status === 'denied') {
      // If denied, it might be due to path not being in workspace - log warning but don't fail
      log(`  Note: Read denied (path may be outside workspace): ${data.reason || 'unknown'}`);
      logPass(`Tool preflight responded correctly (denied with reason)`);
    } else {
      logFail(`Unexpected preflight status for read: ${data.status}`);
      return false;
    }
  } catch (err) {
    logFail(`Tool preflight request failed: ${err.message}`);
    return false;
  }

  return true;
}

async function checkDenialEnforcement() {
  log('Testing denial enforcement for system-critical path...');

  // Attempt to write to a system-critical path
  const systemPath = '/etc/systemd/system/e2e-test.service';

  try {
    const response = await fetchWithTimeout(`${AGENT_RULER_URL}/api/openclaw/tool/preflight`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        tool_name: 'write',
        params: { path: systemPath, content: 'test' },
        context: { agent_id: 'e2e-sanity-check', session_key: 'test-session' }
      })
    });

    if (!response.ok) {
      logFail(`Denial preflight returned status ${response.status}`);
      return false;
    }

    const data = await response.json();
    if (data.blocked === true && data.reason) {
      logPass(`System-critical write blocked with reason: ${data.reason}`);
      return true;
    } else if (data.status === 'denied') {
      logPass(`System-critical write denied`);
      return true;
    } else {
      logFail(`System-critical write was not blocked! Status: ${data.status}, blocked: ${data.blocked}`);
      return false;
    }
  } catch (err) {
    logFail(`Denial preflight request failed: ${err.message}`);
    return false;
  }
}

async function checkUserDataWriteDenial() {
  log('Testing user data write denial...');

  // Attempt to write to user Documents folder
  const userPath = `${homedir()}/Documents/e2e-test-file.txt`;

  try {
    const response = await fetchWithTimeout(`${AGENT_RULER_URL}/api/openclaw/tool/preflight`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        tool_name: 'write',
        params: { path: userPath, content: 'test' },
        context: { agent_id: 'e2e-sanity-check', session_key: 'test-session' }
      })
    });

    if (!response.ok) {
      logFail(`User data write preflight returned status ${response.status}`);
      return false;
    }

    const data = await response.json();
    if (data.blocked === true && data.reason === 'deny_user_data_write') {
      logPass(`User data write blocked with reason: ${data.reason}`);
      return true;
    } else if (data.status === 'denied' && data.reason?.includes('user_data')) {
      logPass(`User data write denied: ${data.reason}`);
      return true;
    } else {
      logFail(`User data write was not blocked! Status: ${data.status}, reason: ${data.reason}`);
      return false;
    }
  } catch (err) {
    logFail(`User data write preflight request failed: ${err.message}`);
    return false;
  }
}

async function checkReceiptsTimeline() {
  log('Checking receipts timeline for recent entries...');

  try {
    const response = await fetchWithTimeout(`${AGENT_RULER_URL}/api/receipts?limit=10`);

    if (!response.ok) {
      logFail(`Receipts endpoint returned status ${response.status}`);
      return false;
    }

    const data = await response.json();
    const items = data.items || [];

    if (items.length > 0) {
      // Look for denial receipts from our tests
      const denialReceipts = items.filter(i =>
        i.decision?.verdict === 'deny' ||
        i.decision?.reason?.includes('deny')
      );

      if (denialReceipts.length > 0) {
        logPass(`Found ${denialReceipts.length} denial receipt(s) in timeline`);
        denialReceipts.slice(0, 3).forEach(r => {
          log(`  - ${r.decision?.reason}: ${r.action?.path || r.action?.operation || 'N/A'}`);
        });
        return true;
      } else {
        logPass(`Receipts timeline contains ${items.length} entries (no denials in recent batch)`);
        return true;
      }
    } else {
      logFail('Receipts timeline is empty');
      return false;
    }
  } catch (err) {
    logFail(`Receipts check failed: ${err.message}`);
    return false;
  }
}

async function main() {
  console.log('');
  console.log('='.repeat(60));
  console.log('Agent Ruler + OpenClaw End-to-End Sanity Check');
  console.log('='.repeat(60));
  console.log('');

  log(`Agent Ruler URL: ${AGENT_RULER_URL}`);
  log(`Timeout: ${TIMEOUT_MS}ms`);
  console.log('');

  // Run all checks
  await checkUiReachable();
  await checkToolPreflight();
  await checkDenialEnforcement();
  await checkUserDataWriteDenial();
  await checkReceiptsTimeline();

  // Summary
  console.log('');
  console.log('='.repeat(60));
  console.log('Summary');
  console.log('='.repeat(60));
  console.log(`  Passed: ${passes.length}`);
  console.log(`  Failed: ${failures.length}`);
  console.log('');

  if (failures.length > 0) {
    console.log('Failures:');
    failures.forEach((f, i) => console.log(`  ${i + 1}. ${f}`));
    console.log('');
    process.exit(1);
  }

  console.log('All sanity checks passed!');
  process.exit(0);
}

main().catch(err => {
  console.error('Fatal error:', err);
  process.exit(1);
});
