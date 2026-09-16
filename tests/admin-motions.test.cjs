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

async function setup({ token = 'test', assigned = false } = {}) {
  class Element {
    constructor(tag = 'div') { this.tag = tag; this.children = []; this.listeners = {}; this.value = ''; }
    append(...children) { for (const child of children) { child.parent = this; this.children.push(child); } }
    replaceChildren(...children) { this.children = []; this.append(...children); }
    add(option) { this.append(option); }
    addEventListener(type, fn) { this.listeners[type] = fn; }
    setAttribute(name, value) { this[name] = value; }
    querySelectorAll(tag) { return this.children.flatMap((child) => [...(child.tag === tag ? [child] : []), ...child.querySelectorAll(tag)]); }
    remove() { this.parent.children = this.parent.children.filter((child) => child !== this); }
    get childElementCount() { return this.children.length; }
    focus() { this.focused = true; }
    reportValidity() { return true; }
  }
  const ids = ['motion-settings-form', 'motion-settings-fields', 'motion-groups', 'reload-motion-settings',
    'motion-settings-status', 'motion-settings-error', 'food-consume-seconds', 'food-speech-seconds',
    'food-duration-seconds', 'delete-motion-file', 'delete-motion'];
  const elements = Object.fromEntries(ids.map((id) => [id, new Element()]));
  const url = '/assets/motions/unused.vrma';
  const config = { files: [{ name: 'unused.vrma', url }], idle_motions: assigned ? [url] : [], emotion_motions: {},
    food_motion: { url: '', consume_at_ms: 2505, speech_start_ms: 4200, duration_ms: 13440 } };
  const requests = [], messages = [], confirmations = [];
  const controls = { confirm: true, fail: false };
  const context = vm.createContext({
    document: { querySelector: (selector) => elements[selector.slice(1)], createElement: (tag) => new Element(tag) },
    Option: function (label, value) { const option = new Element('option'); option.textContent = label; option.value = value; return option; },
    window: { confirm: (text) => { confirmations.push(text); return controls.confirm; } },
    motionFileName: (value) => value.split('/').at(-1),
    fetch: async (path, options = {}) => {
      if (!options.method) return { ok: true, json: async () => config };
      requests.push({ path, method: options.method, body: JSON.parse(options.body) });
      return { ok: !controls.fail };
    },
  });
  vm.runInContext(`${source}\nthis.init = initMotionSettings;`, context);
  const load = context.init({ token, adminUrl: (path) => path, readError: async () => '処理失敗',
    setMessage: (_status, _error, text, error) => messages.push({ text, error }) });
  await load();
  return { elements, config, url, requests, messages, confirmations, controls };
}

test('消去と発話開始の時刻を独立して読み込み保存する', async () => {
  const { elements, requests } = await setup();
  assert.equal(elements['food-consume-seconds'].value, '2.505');
  assert.equal(elements['food-speech-seconds'].value, '4.2');
  for (const speech of ['0', '20.123']) {
    elements['food-speech-seconds'].value = speech;
    await elements['motion-settings-form'].listeners.submit({ preventDefault() {} });
    assert.equal(requests.at(-1).body.food_motion.speech_start_ms, Math.round(Number(speech) * 1000));
    assert.equal(requests.at(-1).body.food_motion.consume_at_ms, 2505);
    assert.equal(requests.at(-1).body.food_motion.duration_ms, 13440);
  }
  elements['food-speech-seconds'].value = '-1';
  await elements['motion-settings-form'].listeners.submit({ preventDefault() {} });
  assert.equal(requests.length, 2);
});

test('ファイル削除は確認後に送信し成功時だけ一覧から外す', async () => {
  const { elements, url, requests, confirmations, controls } = await setup();
  const picker = elements['delete-motion-file'], remove = elements['delete-motion'];
  picker.value = url;
  controls.confirm = false;
  await remove.listeners.click();
  assert.equal(requests.length, 0);
  assert.match(confirmations[0], /unused\.vrma/);
  controls.confirm = true; controls.fail = true;
  await remove.listeners.click();
  assert.equal(picker.children.length, 2);
  assert.equal(picker.value, url);
  controls.fail = false;
  await remove.listeners.click();
  assert.deepEqual(requests.at(-1), { path: '/api/admin/motions', method: 'DELETE', body: { url } });
  assert.equal(picker.children.length, 1);
  assert.equal(picker.value, '');
  assert.equal(elements['food-speech-seconds'].value, '4.2');
});

test('候補内のファイルと認証なしの削除は送信しない', async () => {
  for (const options of [{ assigned: true }, { token: '' }]) {
    const { elements, requests, confirmations, url } = await setup(options);
    elements['delete-motion-file'].value = url;
    await elements['delete-motion'].listeners.click();
    assert.equal(requests.length, 0);
    assert.equal(confirmations.length, 0);
  }
});
