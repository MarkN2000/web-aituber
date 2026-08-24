const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const root = path.resolve(__dirname, "..");
const adminHtml = fs.readFileSync(path.join(root, "web", "admin.html"), "utf8");
const adminSource = fs.readFileSync(path.join(root, "web", "js", "admin.js"), "utf8");
const drawSource = fs.readFileSync(path.join(root, "web", "js", "draw.js"), "utf8");
const stabilizationSource = drawSource.slice(
  drawSource.indexOf("function stabilizePoint"),
  drawSource.indexOf("function drawLine"),
);
const stabilizationContext = vm.createContext({ Math });
vm.runInContext(
  `const STABILIZATION_TIME_CONSTANT_MS = 8; ${stabilizationSource}; this.stabilize = stabilizePoint;`,
  stabilizationContext,
);

test("手ブレ補正0は入力点を変更しない", () => {
  const point = stabilizationContext.stabilize({ x: 0, y: 0 }, { x: 100, y: 50 }, 16, 0);

  assert.equal(point.x, 100);
  assert.equal(point.y, 50);
});

test("手ブレ補正を強くすると入力点の変化を小さくする", () => {
  const weak = stabilizationContext.stabilize({ x: 0, y: 0 }, { x: 100, y: 0 }, 16, 2);
  const strong = stabilizationContext.stabilize({ x: 0, y: 0 }, { x: 100, y: 0 }, 16, 8);

  assert.ok(weak.x > strong.x);
  assert.ok(strong.x > 0);
  assert.ok(weak.x < 100);
});

test("時間基準の補正は同じ経過時間で同じ位置になる", () => {
  const once = stabilizationContext.stabilize({ x: 0, y: 0 }, { x: 100, y: 0 }, 16, 4);
  const first = stabilizationContext.stabilize({ x: 0, y: 0 }, { x: 100, y: 0 }, 8, 4);
  const twice = stabilizationContext.stabilize(first, { x: 100, y: 0 }, 8, 4);

  assert.ok(Math.abs(once.x - twice.x) < 1e-10);
});

test("描画画面は管理設定を取得し、まとめられた入力点へ補正を適用する", () => {
  assert.match(drawSource, /fetch\(`\$\{eventBasePath\}\/api\/display-config`, \{ cache: "no-store" \}\)/);
  assert.match(drawSource, /drawingStabilization = stabilization/);
  assert.match(drawSource, /void loadDrawingConfig\(\)\s+\.then/);
  assert.doesNotMatch(drawSource, /await loadDrawingConfig\(\)/);
  assert.match(drawSource, /else pendingDrawingStabilization = stabilization/);
  assert.match(drawSource, /event\.getCoalescedEvents\?\.\(\)/);
  assert.match(drawSource, /const stabilized = stabilizePoint\(/);
  assert.match(drawSource, /if \(activeTool === "bucket"\) \{[\s\S]*fillAt\(point\);[\s\S]*return;/);
});

test("管理画面は0から10の手ブレ補正を保存する", () => {
  assert.match(adminHtml, /id="drawing-stabilization" type="range" min="0" max="10" step="1" value="3"/);
  assert.match(adminSource, /\/api\/admin\/drawing-stabilization/);
  assert.match(adminSource, /body: JSON\.stringify\(\{ stabilization: Number\(elements\.drawingStabilization\.value\) \}\)/);
  assert.match(adminSource, /描画画面を再読み込みすると反映されます/);
});
