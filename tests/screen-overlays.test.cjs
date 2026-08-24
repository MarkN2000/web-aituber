const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const root = path.resolve(__dirname, "..");
const adminHtml = fs.readFileSync(path.join(root, "web", "admin.html"), "utf8");
const adminScript = fs.readFileSync(path.join(root, "web", "js", "admin.js"), "utf8");
const mainHtml = fs.readFileSync(path.join(root, "web", "main.html"), "utf8");
const mainScript = fs.readFileSync(path.join(root, "web", "js", "main.js"), "utf8");
const style = fs.readFileSync(path.join(root, "web", "style.css"), "utf8");

test("管理画面は四隅の画面オーバーレイスロットと倍率操作を表示する", () => {
  for (const slot of ["top-left", "top-right", "bottom-left", "bottom-right"]) {
    assert.match(adminHtml, new RegExp(`data-screen-overlay-slot="${slot}"`, "g"));
  }
  assert.match(adminHtml, /data-screen-overlay-scale type="range" min="1" max="100" step="1"/);
  assert.match(adminHtml, /data-screen-overlay-scale-number type="number" min="1" max="100" step="1"/);
  assert.match(adminScript, /\/api\/admin\/screen-overlays\/\$\{overlay\.slot\}/);
  assert.match(adminScript, /\/api\/admin\/screen-overlays\/\$\{overlay\.slot\}\/scale/);
  assert.match(adminScript, /body: JSON\.stringify\(\{ scale \}\)/);
  assert.match(adminScript, /syncScreenOverlayScaleFromRange/);
  assert.match(adminScript, /syncScreenOverlayScaleFromNumber/);
  assert.match(adminScript, /showCurrentScreenOverlay\(overlay, \{ scale: Number\(overlay\.scale\.value\) \}\)/);
});

test("メイン画面はVRMより前、UIより後ろのオーバーレイレイヤーを持つ", () => {
  assert.match(mainHtml, /<canvas id="vrm-canvas"[\s\S]*<div id="screen-overlays"[\s\S]*<p id="viewer-message"/);
  assert.match(mainScript, /const SCREEN_OVERLAY_SLOTS = \[/);
  assert.match(mainScript, /image\.style\.objectPosition = origin/);
  assert.match(mainScript, /image\.style\.transformOrigin = origin/);
  assert.match(mainScript, /image\.style\.transform = `scale\(/);
  assert.match(style, /#screen-overlays \{[\s\S]*z-index: 1;[\s\S]*pointer-events: none;/);
  assert.match(style, /\.main-bottom \{[\s\S]*z-index: 3;/);
});
