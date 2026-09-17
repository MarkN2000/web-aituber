const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/admin.js"), "utf8");
const handler = source.slice(source.indexOf("async function clearTtsCache()"), source.indexOf("function releasePreview()"));

test("音声キャッシュ削除は二重送信を防ぎ成功と失敗を表示する", async () => {
  const elements = { clearTtsCache: { disabled: false } };
  let finish;
  let requests = 0;
  const context = vm.createContext({
    token: "test", elements,
    adminUrl: (url) => `${url}?token=test`,
    readError: async () => "削除できません",
    setMessage: (_status, _error, message, failed) => { context.message = message; context.failed = failed; },
    fetch: (url, options) => {
      assert.equal(url, "/api/admin/tts-cache?token=test");
      assert.equal(options.method, "DELETE");
      requests++;
      return new Promise((resolve) => { finish = resolve; });
    },
  });
  vm.runInContext(handler, context);
  const pending = context.clearTtsCache();
  assert.equal(elements.clearTtsCache.disabled, true);
  await context.clearTtsCache();
  assert.equal(requests, 1);
  finish({ ok: true });
  await pending;
  assert.match(context.message, /削除しました/);
  assert.equal(elements.clearTtsCache.disabled, false);

  const failed = context.clearTtsCache();
  finish({ ok: false });
  await failed;
  assert.equal(context.message, "削除できません");
  assert.equal(context.failed, true);
  assert.equal(elements.clearTtsCache.disabled, false);
  context.token = "";
  await context.clearTtsCache();
  assert.equal(requests, 2);
});
