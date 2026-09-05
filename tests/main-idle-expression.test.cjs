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
