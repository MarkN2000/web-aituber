const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const root = path.join(__dirname, "..");
const adminHtml = fs.readFileSync(path.join(root, "web", "admin.html"), "utf8");
const mainHtml = fs.readFileSync(path.join(root, "web", "main.html"), "utf8");
const style = fs.readFileSync(path.join(root, "web", "style.css"), "utf8");
const mainScript = fs.readFileSync(path.join(root, "web", "js", "main.js"), "utf8");

test("準備中画像の全画面スタイルはメイン画面だけに適用する", () => {
  assert.match(mainHtml, /<body class="main-page">/);
  assert.match(mainHtml, /<img id="preparation-image"/);
  assert.match(adminHtml, /<input id="preparation-image" type="file"/);
  assert.match(style, /\.main-page #preparation-image\s*\{/);
  assert.equal((style.match(/#preparation-image/g) || []).length, 1);
});

test("管理画面は修正後のスタイルシートを読み込む", () => {
  assert.match(adminHtml, /\/static\/style\.css\?v=24/);
});

test("準備中画像がない場合はステージを黒くする", () => {
  assert.match(mainScript, /function enterPreparationMode\(config\) \{[\s\S]*elements\.stage\.style\.backgroundColor = "#000";/);
  assert.match(mainScript, /if \(config\.preparation_image_url\)[\s\S]*elements\.preparationImage\.hidden = true;/);
});
