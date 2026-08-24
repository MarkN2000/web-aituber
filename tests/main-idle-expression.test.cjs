const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/main.js"), "utf8");
const setTurnSource = source.slice(
  source.indexOf("function setTurn"),
  source.indexOf("function setEmotion"),
);
const cleanTurnSource = source.slice(
  source.indexOf("function cleanTurn"),
  source.indexOf("function cancelTurnAudio"),
);

test("待機状態へ移るとneutral表情と待機モーションへ戻す", () => {
  const calls = [];
  const context = vm.createContext({ calls });
  vm.runInContext(`
    const calls = this.calls;
    let currentTurn = { turn_id: "turn-1" };
    const viewer = {
      setIdleExpression() { calls.push("expression"); },
      resumeIdle() { calls.push("motion"); },
    };
    const elements = {
      answer: { hidden: false },
      answerText: { textContent: "" },
      loader: { hidden: false },
      panel: { hidden: false },
    };
    function clearAnswer() { calls.push("answer"); }
    function applyPendingViewerConfig() { calls.push("config"); }
    ${setTurnSource}
    this.setTurn = setTurn;
  `, context);

  context.setTurn(undefined);

  assert.deepEqual(calls, ["expression", "motion", "answer", "config"]);
});

test("回答終了処理は表情を直接解除せず待機状態へ移る", () => {
  assert.match(cleanTurnSource, /viewer\?\.clearFoodProp\(\);\s+setTurn\(undefined\);/);
  assert.doesNotMatch(cleanTurnSource, /setEmotion\("neutral"\)/);
});

test("メイン画面は更新したVRMビューアーを読み込む", () => {
  assert.match(source, /import\("\.\/vrm-viewer\.js\?v=18"\)/);
});
