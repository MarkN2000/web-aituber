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
    connection: "connected",
    motionFileName: "happy.vrma",
    motionKind: "emotion",
    expression: "happy",
    expressionSupport: "supported",
    foodAction: "consuming",
  });

  assert.equal(element.hidden, false);
  assert.equal(
    element.textContent,
    "接続: 接続済み\nモーション: happy.vrma\n種別: 感情\n要求表情: happy\n表情対応: あり\n食事動作: 消費中",
  );
});

test("身体モーションがない状態を明示する", () => {
  const element = { hidden: true, textContent: "" };

  context.render(element, {
    connection: "reconnecting",
    expression: "neutral",
    expressionSupport: "base",
    foodAction: "none",
  });

  assert.equal(
    element.textContent,
    "接続: 再接続中\nモーション: なし\n種別: なし\n要求表情: neutral\n表情対応: 基本状態\n食事動作: なし",
  );
});

test("食事モーションのファイル名と種別を表示する", () => {
  const element = { hidden: true, textContent: "" };
  context.render(element, { motionFileName: "eat2.vrma", motionKind: "food" });
  assert.match(element.textContent, /モーション: eat2.vrma\n種別: 食事/);
});

test("接続中・未対応表情・食事画像読込失敗を表示する", () => {
  const element = { hidden: true, textContent: "" };

  context.render(element, {
    connection: "connecting",
    expression: "sad",
    expressionSupport: "unsupported",
    foodAction: "failed",
  });

  assert.equal(
    element.textContent,
    "接続: 接続中\nモーション: なし\n種別: なし\n要求表情: sad\n表情対応: 未対応\n食事動作: 読込失敗",
  );
});
