const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "web", "admin.html"), "utf8");
const script = fs.readFileSync(path.join(root, "web", "js", "admin.js"), "utf8");

test("管理画面にはバージョンとアップデート確認操作を表示する", () => {
  assert.match(html, /id="current-app-version"/);
  assert.match(html, /id="check-update"[^>]*>アップデートを確認</);
  assert.match(html, /id="update-status"[^>]*role="status"/);
  assert.match(html, /id="update-error"[^>]*role="alert"/);
});

test("アップデートは確認後に同意を取り、再起動した版を確認する", () => {
  assert.match(script, /fetch\(adminUrl\("\/api\/admin\/update"\), \{ cache: "no-store" \}\)/);
  assert.match(script, /window\.confirm\(`v\$\{result\.latest_version\}へアップデートして再起動しますか/);
  assert.match(script, /fetch\(adminUrl\("\/api\/admin\/update"\), \{ method: "POST" \}\)/);
  assert.match(script, /result\.current_version === targetVersion/);
});

test("管理画面にQRなしのデバッグ用メイン画面リンクを表示する", () => {
  assert.match(html, /id="event-debug-url"/);
  assert.match(html, /data-copy-event-url="debug"/);
  assert.doesNotMatch(html, /data-qr-event-url="debug"/);
  assert.match(script, /debug: base \? `\$\{base\}\?debug` : ""/);
});

test("管理画面でアンチエイリアスをON・OFFして保存する", () => {
  assert.match(html, /id="model-antialias" type="checkbox" checked/);
  assert.match(html, /id="save-model-antialias"/);
  assert.match(script, /fetch\(adminUrl\("\/api\/admin\/model-antialias"\)/);
  assert.match(script, /antialias: elements\.antialias\.checked/);
});
