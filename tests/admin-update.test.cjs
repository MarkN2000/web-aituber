const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "web", "admin.html"), "utf8");
const script = fs.readFileSync(path.join(root, "web", "js", "admin.js"), "utf8");
const routes = fs.readFileSync(path.join(root, "src", "routes.rs"), "utf8");
const update = fs.readFileSync(path.join(root, "src", "update.rs"), "utf8");
const updater = fs.readFileSync(path.join(root, "src", "bin", "web-aituber-updater.rs"), "utf8");

test("管理画面にはバージョンとアップデート確認操作を表示する", () => {
  assert.match(html, /id="current-app-version"/);
  assert.match(html, /id="check-update"[^>]*>アップデートを確認</);
  assert.match(html, /id="update-status"[^>]*role="status"/);
  assert.match(html, /id="update-error"[^>]*role="alert"/);
});

test("自己更新は接続を閉じて終了期限を設け、進行状況をログへ残す", () => {
  assert.match(routes, /state\.shutdown\.subscribe\(\)/);
  assert.match(routes, /sender\.send\(Message::Close\(None\)\)/);
  assert.match(routes, /std::thread::spawn/);
  assert.match(updater, /PARENT_GRACEFUL_EXIT_TIMEOUT: Duration = Duration::from_secs\(10\)/);
  assert.match(updater, /terminate_parent\(parent_pid\)/);
  assert.match(updater, /UPDATE_LOG_FILE_NAME: &str = "update\.log"/);
  assert.match(updater, /append_update_log\(log_path/);
});

test("Unixでは外部アップデーターを親の端末セッションから分離する", () => {
  assert.match(update, /command\.pre_exec/);
  assert.match(update, /libc::setsid\(\)/);
  assert.match(updater, /detach_from_parent_session\(\)/);
  assert.match(updater, /libc::setsid\(\)/);
});

test("アップデートは確認後に同意を取り、再起動した版を確認する", () => {
  assert.match(script, /fetch\(adminUrl\("\/api\/admin\/update"\), \{ cache: "no-store" \}\)/);
  assert.match(script, /window\.confirm\(`v\$\{result\.latest_version\}へアップデートして再起動しますか/);
  assert.match(script, /fetch\(adminUrl\("\/api\/admin\/update"\), \{ method: "POST" \}\)/);
  assert.match(script, /result\.current_version === targetVersion/);
  assert.match(script, /AbortSignal\.timeout\(timeoutMs\)/);
  assert.match(script, /UPDATE_RECONNECT_TOTAL_TIMEOUT_MS = 120_000/);
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

test("準備中画像がなくても準備中モードを保存できる", () => {
  assert.match(html, /画像がない場合は黒画面になります。/);
  assert.match(html, /\/static\/js\/admin\.js\?v=37/);
  assert.doesNotMatch(script, /missingImage/);
  assert.doesNotMatch(script, /if \(enabled && !preparationImageAsset\.currentExists\) return;/);
});
