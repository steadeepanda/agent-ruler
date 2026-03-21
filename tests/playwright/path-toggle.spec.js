const fs = require('node:fs/promises');
const path = require('node:path');
const { test, expect } = require('@playwright/test');

const BASE_URL = process.env.AR_UI_URL || 'http://127.0.0.1:4636';

async function readRunnerZoneWorkspaceText(page) {
  await page.waitForSelector('#runners-grid .form-hint');
  return page.evaluate(() => {
    const hints = Array.from(document.querySelectorAll('#runners-grid .form-hint'));
    const zoneHint = hints.find((node) =>
      node.textContent && node.textContent.includes('Zone 0 (workspace):')
    );
    return zoneHint ? zoneHint.textContent.trim() : '';
  });
}

async function readRuntimeWorkspacePathText(page) {
  await page.waitForSelector('#runtime-hide-paths');
  return page.evaluate(() => {
    const rows = Array.from(document.querySelectorAll('.table tbody tr'));
    const workspaceRow = rows.find((row) => {
      const firstCell = row.querySelector('td');
      return firstCell && firstCell.textContent && firstCell.textContent.includes('Workspace');
    });
    if (!workspaceRow) return '';
    const cells = workspaceRow.querySelectorAll('td');
    return cells.length > 1 ? (cells[1].textContent || '').trim() : '';
  });
}

test('global path label toggle applies to Import/Export and Runners views', async ({ page }, testInfo) => {
  const consoleLines = [];
  const network = [];

  page.on('console', (msg) => {
    consoleLines.push(`[${msg.type()}] ${msg.text()}`);
  });

  page.on('requestfinished', (request) => {
    network.push({ method: request.method(), url: request.url() });
  });

  await page.goto(`${BASE_URL}/settings`, { waitUntil: 'networkidle' });
  const toggle = page.locator('#settings-runtime-path-labels');
  await expect(toggle).toBeVisible();

  await toggle.check();
  await page.goto(`${BASE_URL}/runtime`, { waitUntil: 'networkidle' });
  const runtimeWorkspaceOn = await readRuntimeWorkspacePathText(page);
  expect(runtimeWorkspaceOn.includes('WORKSPACE_PATH')).toBeFalsy();
  expect(runtimeWorkspaceOn.startsWith('/')).toBeTruthy();
  await page.locator('#runtime-hide-paths').check();
  await expect(page.locator('text=[hidden]')).toBeVisible();

  await page.goto(`${BASE_URL}/files`, { waitUntil: 'networkidle' });
  const filesWorkspace = page.locator('#zone-path-workspace');
  await expect(filesWorkspace).toBeVisible();
  await expect(filesWorkspace).toContainText('WORKSPACE_PATH');

  await page.goto(`${BASE_URL}/runners`, { waitUntil: 'networkidle' });
  const runnersZoneOn = await readRunnerZoneWorkspaceText(page);
  expect(runnersZoneOn.includes('WORKSPACE_PATH')).toBeTruthy();
  await page.screenshot({ path: path.join(testInfo.outputDir, 'toggle-on-runners.png'), fullPage: true });

  await page.goto(`${BASE_URL}/settings`, { waitUntil: 'networkidle' });
  await toggle.uncheck();

  await page.goto(`${BASE_URL}/files`, { waitUntil: 'networkidle' });
  const filesWorkspaceOff = (await filesWorkspace.textContent() || '').trim();
  expect(filesWorkspaceOff.includes('WORKSPACE_PATH')).toBeFalsy();
  expect(filesWorkspaceOff.startsWith('/')).toBeTruthy();

  await page.goto(`${BASE_URL}/runners`, { waitUntil: 'networkidle' });
  const runnersZoneOff = await readRunnerZoneWorkspaceText(page);
  expect(runnersZoneOff.includes('WORKSPACE_PATH')).toBeFalsy();
  expect(runnersZoneOff.includes('/')).toBeTruthy();
  await page.screenshot({ path: path.join(testInfo.outputDir, 'toggle-off-runners.png'), fullPage: true });

  await fs.writeFile(path.join(testInfo.outputDir, 'console.log'), `${consoleLines.join('\n')}\n`, 'utf8');
  await fs.writeFile(path.join(testInfo.outputDir, 'network.json'), `${JSON.stringify(network, null, 2)}\n`, 'utf8');
});
