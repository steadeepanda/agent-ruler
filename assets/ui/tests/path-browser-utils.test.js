const utils = require('../path-browser-utils.js');

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(message || `Expected "${expected}", got "${actual}"`);
  }
}

function runTests() {
  console.log('Running path browser utils tests...');
  assertEqual(
    utils.normalizeBrowserPrefix('..'),
    '',
    'Root should stay at root when prefix is ..'
  );
  assertEqual(
    utils.normalizeBrowserPrefix('../etc'),
    'etc',
    'Parent segments should be dropped'
  );
  assertEqual(
    utils.normalizeBrowserPrefix('folder/../sub'),
    'sub',
    'Intermediate parent traversals should collapse'
  );
  assertEqual(
    utils.normalizeBrowserPrefix('../../../../'),
    '',
    'Deep parent traversal should clamp to empty string'
  );
  console.log('All path browser utils tests passed.');
}

if (require.main === module) {
  try {
    runTests();
  } catch (err) {
    console.error(err);
    process.exit(1);
  }
}

module.exports = { runTests };
