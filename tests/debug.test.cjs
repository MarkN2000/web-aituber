const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/debug.js"), "utf8")
  .replaceAll("export function ", "function ");
const context = vm.createContext({ URL, URLSearchParams });
vm.runInContext(
  `${source}\nthis.debugEnabled = isDebugEnabled; this.fileName = motionFileName; this.render = renderDebugState;`,
  context,
);

test("debugパラメータは値を問わずデバッグ表示を有効にする", () => {
  assert.equal(context.debugEnabled("?debug"), true);
  assert.equal(context.debugEnabled("?debug="), true);
  assert.equal(context.debugEnabled("?debug=food-prop"), true);
  assert.equal(context.debugEnabled("?debug=false"), true);
  assert.equal(context.debugEnabled("?mode=debug"), false);
  assert.equal(context.debugEnabled(""), false);
});

test("モーションURLからクエリを除いたファイル名を取得する", () => {
  assert.equal(
    context.fileName("/assets/motions/VRMA%2001.vrma?v=123", "http://localhost:3000/"),
    "VRMA 01.vrma",
  );
  assert.equal(
    context.fileName("/assets/motions/idle%ZZ.vrma", "http://localhost:3000/"),
    "idle%ZZ.vrma",
  );
});

test("デバッグ状態はモーション種別と要求表情を表示する", () => {
  const element = { hidden: true, textContent: "" };

  context.render(element, {
    motionFileName: "happy.vrma",
    motionKind: "emotion",
    expression: "happy",
  });

  assert.equal(element.hidden, false);
  assert.equal(element.textContent, "モーション: happy.vrma\n種別: 感情\n要求表情: happy");
});

test("身体モーションがない状態を明示する", () => {
  const element = { hidden: true, textContent: "" };

  context.render(element, { expression: "neutral" });

  assert.equal(element.textContent, "モーション: なし\n種別: なし\n要求表情: neutral");
});
