const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '../web/js/admin-motions.js'), 'utf8')
  .replace(/^import .*;\r?\n/gm, '').replace(/export /g, '');
const context = vm.createContext({});
vm.runInContext(`${source}\nthis.convert = secondsToMilliseconds;`, context);

test('食事時刻はミリ秒の丸め誤差を避けて変換する', () => {
  for (const [input, expected] of [['0', 0], ['3.505', 3505], ['1.001', 1001], ['14.44', 14440]]) {
    assert.equal(context.convert(input), expected);
  }
  for (const invalid of ['', '-1', 'NaN', 'Infinity', '0.0001', '1e5', '9007199254740992']) {
    assert.throws(() => context.convert(invalid));
  }
});
