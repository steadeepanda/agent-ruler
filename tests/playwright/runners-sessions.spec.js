const fs = require('node:fs/promises');
const path = require('node:path');
const { test, expect } = require('@playwright/test');

const BASE_URL = process.env.AR_UI_URL || 'http://127.0.0.1:4636';

async function postJson(request, endpoint, data) {
  const response = await request.post(`${BASE_URL}${endpoint}`, { data });
  expect(response.ok(), `${endpoint} should return 2xx`).toBeTruthy();
  return await response.json();
}

test('Runners session explorer filters, searches, and shows thread details', async ({ page, request }, testInfo) => {
  await postJson(request, '/api/sessions/telegram/resolve', {
    runner_kind: 'claudecode',
    chat_id: '-1007001',
    thread_id: 311,
    message_anchor_id: 9001,
    title: 'Design review sync'
  });
  await postJson(request, '/api/sessions/telegram/resolve', {
    runner_kind: 'claudecode',
    chat_id: '-1007001',
    thread_id: 312,
    message_anchor_id: 9002,
    title: 'Bug triage lane'
  });
  await postJson(request, '/api/runners/opencode/tool/preflight', {
    tool_name: 'write',
    params: {
      path: 'playwright-session-seed.txt',
      content: 'seed'
    },
    context: {
      agent_id: 'main',
      session_key: 'opencode-playwright-session'
    }
  });

  const consoleLines = [];
  page.on('console', (msg) => {
    consoleLines.push(`[${msg.type()}] ${msg.text()}`);
  });

  await page.goto(`${BASE_URL}/runners`, { waitUntil: 'networkidle' });
  await expect(page.locator('#runner-sessions-search')).toBeVisible();
  await expect(page.locator('#runner-sessions-list')).toContainText('Design review sync');
  await expect(page.locator('#runner-sessions-list')).toContainText('thread 311');
  await expect(page.locator('#runner-sessions-list')).toContainText('telegram');

  await page.getByRole('tab', { name: 'Claude Code' }).click();
  await page.selectOption('#runner-sessions-channel', 'telegram');
  await page.fill('#runner-sessions-search', 'Design review');
  await page.waitForTimeout(350);

  const sessionRows = page.locator('#runner-sessions-list .list-item');
  await expect(sessionRows).toHaveCount(1);
  await expect(sessionRows.first()).toContainText('Claude Code');
  await expect(sessionRows.first()).toContainText('thread 311');

  await sessionRows.first().getByRole('button', { name: 'Details' }).click();
  await expect(page.locator('#modal-title')).toHaveText('Session Details');
  await expect(page.locator('#modal-body')).toContainText('Design review sync');
  await expect(page.locator('#modal-body')).toContainText('Telegram Thread');
  await expect(page.locator('#modal-body')).toContainText('311');

  await page.screenshot({ path: path.join(testInfo.outputDir, 'runners-sessions-panel.png'), fullPage: true });
  await fs.writeFile(path.join(testInfo.outputDir, 'console.log'), `${consoleLines.join('\n')}\n`, 'utf8');
});
