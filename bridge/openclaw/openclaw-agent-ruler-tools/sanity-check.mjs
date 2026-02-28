#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const sourcePath = path.join(__dirname, 'index.ts');
const source = readFileSync(sourcePath, 'utf8');

const failures = [];

const schemaCount = (source.match(/\bschema\s*:/g) || []).length;
if (schemaCount > 0) {
  failures.push(`found ${schemaCount} deprecated schema field(s); use parameters instead`);
}

const parametersCount = (source.match(/\bparameters\s*:/g) || []).length;
if (parametersCount < 4) {
  failures.push(`expected at least 4 parameters schemas, found ${parametersCount}`);
}

const optionalRegisterHelper = /function\s+registerOptionalTool\s*\([\s\S]*?registerTool\(tool,\s*\{\s*optional\s*:\s*true\s*}\)/m;
if (!optionalRegisterHelper.test(source)) {
  failures.push('registerOptionalTool helper does not call registerTool(tool, { optional: true })');
}

if (failures.length > 0) {
  console.error('OpenClaw adapter sanity check failed:');
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log('OpenClaw adapter sanity check passed.');
