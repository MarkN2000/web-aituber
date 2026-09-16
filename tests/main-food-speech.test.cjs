const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/main.js"), "utf8");
const clearAnswerSource = source.slice(
  source.indexOf("function clearAnswer"),
  source.indexOf("function showCurrentSources"),
);
const setTurnSource = source.slice(
  source.indexOf("function setTurn"),
  source.indexOf("function setEmotion"),
);
const cleanTurnSource = source.slice(
  source.indexOf("function cleanTurn"),
  source.indexOf("function cancelTurnAudio"),
);
const cancelTurnAudioSource = source.slice(
  source.indexOf("function cancelTurnAudio"),
  source.indexOf("function connect"),
);
const handleServerEventSource = source.slice(
  source.indexOf("function handleServerEvent"),
  source.indexOf("function endEventAccess"),
);
const receiveSegmentSource = source.slice(
  source.indexOf("function receiveSegment"),
  source.indexOf("function onAudioStart"),
);
const onAudioEndSource = source.slice(
  source.indexOf("function onAudioEnd"),
  source.indexOf("async function startMain"),
);
const onAudioStartSource = source.slice(source.indexOf("function onAudioStart"), source.indexOf("function onAudioEnd"));
const motionSource = fs.readFileSync(path.join(__dirname, "../web/js/motion.js"), "utf8").replace(/export /g, "");

function loadContext() {
  const calls = [];
  const context = vm.createContext({ calls });
  vm.runInContext(`
    const calls = this.calls;
    let currentTurn;
    let motionPlayedForTurn;
    let currentSourceButton;
    const receivedTurns = new Set();
    const displayConfig = {};
    const elements = {
      panel: { hidden: false },
      loader: { hidden: false },
      answer: { hidden: true },
      answerText: { textContent: "" },
    };
    const viewer = {
      clearFoodProp() { calls.push("clearFood"); },
      playFoodAction() { calls.push("food"); },
      stopLipSync() { calls.push("stopLip"); },
      setIdleExpression() { calls.push("idleExpression"); },
      resumeIdle() { calls.push("idleMotion"); },
      startLipSync() {},
      playEmotionMotion(emotion) { calls.push(["motion", emotion]); },
    };
    const queue = {
      cancelTurn(turnId) { calls.push(["cancel", turnId]); },
      enqueue(item) { calls.push(["enqueue", item.turnId, item.sequence]); },
    };
    const backgroundMusic = {
      setDucked(value) { calls.push(["duck", value]); },
    };
    function setEmotion(value) { calls.push(["expression", value]); }
    function applyPendingViewerConfig() { calls.push("config"); }
    ${clearAnswerSource}
    ${setTurnSource}
    ${cleanTurnSource}
    ${cancelTurnAudioSource}
    function historyView() {}
    historyView.render = () => {};
    function refreshDisplayConfig() {}
    function endEventAccess() {}
    function showError() {}
    function showCurrentSources() {}
    ${handleServerEventSource}
    ${receiveSegmentSource}
    ${motionSource}
    ${onAudioStartSource}
    ${onAudioEndSource}
    this.handle = handleServerEvent;
    this.audioEnd = onAudioEnd;
    this.audioStart = onAudioStart;
    this.currentTurn = () => currentTurn;
    this.received = (turnId) => receivedTurns.has(turnId);
  `, context);
  return { context, calls };
}

function state(turnId) {
  return { type: "state", turn: { turn_id: turnId, question: "質問", status: "speaking" } };
}

function segment(turnId) {
  return {
    type: "segment",
    turn_id: turnId,
    sequence: 0,
    text: "感想です",
    kind: "answer",
    emotion: "happy",
    motion: null,
    audio_url: "/audio/test.webm",
    duration_ms: 1000,
    is_last: true,
  };
}

function audioItem(turnId) {
  return { turnId, meta: { is_last: true } };
}

test("音声開始時に指定感情のモーションを回答あたり最大1回だけ再生する", () => {
  const { context, calls } = loadContext();
  const start = (turnId, motion, kind = "answer") => context.audioStart({ meta: { ...segment(turnId), motion, kind } });
  start("turn-1", null);
  start("turn-1", "sad", "filler");
  start("turn-1", "invalid");
  assert.deepEqual(calls.filter((call) => call[0] === "motion"), []);
  start("turn-1", "happy");
  start("turn-1", "sad");
  start("turn-2", "sad");
  assert.deepEqual(calls.filter((call) => call[0] === "motion").map((call) => Array.from(call)), [["motion", "happy"], ["motion", "sad"]]);
});

test("Completeが先でも最終音声の終了まで食事演出を片付けない", () => {
  const { context, calls } = loadContext();
  context.handle(state("food-1"));
  context.handle({ type: "food_action", image_url: "/food/1.webp", consume_at_ms: 1000, duration_ms: 3000 });
  context.handle(segment("food-1"));
  context.handle({ type: "complete", turn_id: "food-1" });

  assert.equal(calls.filter((call) => call === "clearFood").length, 0);
  assert.equal(context.currentTurn().serverCompleted, true);

  context.audioEnd(audioItem("food-1"));

  assert.equal(calls.filter((call) => call === "clearFood").length, 1);
  assert.equal(context.currentTurn(), undefined);
});

test("最終音声が先でもCompleteまで食事演出を片付けない", () => {
  const { context, calls } = loadContext();
  context.handle(state("food-1"));
  context.handle({ type: "food_action", image_url: "/food/1.webp", consume_at_ms: 1000, duration_ms: 3000 });
  context.handle(segment("food-1"));
  context.audioEnd(audioItem("food-1"));

  assert.equal(context.received("food-1"), false);
  assert.equal(calls.filter((call) => call === "clearFood").length, 0);

  context.handle({ type: "complete", turn_id: "food-1" });

  assert.equal(calls.filter((call) => call === "clearFood").length, 1);
  assert.equal(context.currentTurn(), undefined);
});

test("音声がない投稿はCompleteで片付ける", () => {
  const { context, calls } = loadContext();
  context.handle(state("turn-1"));
  context.handle({ type: "complete", turn_id: "turn-1" });

  assert.equal(calls.filter((call) => call === "clearFood").length, 1);
  assert.equal(context.currentTurn(), undefined);
});

test("中断とエラーはCompleteを待たずに食事演出を片付ける", () => {
  for (const type of ["cancelled", "error"]) {
    const { context, calls } = loadContext();
    context.handle(state("food-1"));
    context.handle({ type: "food_action", image_url: "/food/1.webp", consume_at_ms: 1000, duration_ms: 3000 });
    context.handle(segment("food-1"));
    context.handle({ type, turn_id: "food-1", message: "失敗" });

    assert.equal(calls.filter((call) => call === "clearFood").length, 1, type);
    assert.equal(context.currentTurn(), undefined, type);
  }
});

test("別ターンへ移った後の古いCompleteは現在の食事演出へ影響しない", () => {
  const { context, calls } = loadContext();
  context.handle(state("old"));
  context.handle({ type: "food_action", image_url: "/food/old.webp", consume_at_ms: 1000, duration_ms: 3000 });
  context.handle(segment("old"));
  context.handle(state("new"));
  const clearCount = calls.filter((call) => call === "clearFood").length;

  context.handle({ type: "complete", turn_id: "old" });

  assert.equal(calls.filter((call) => call === "clearFood").length, clearCount);
  assert.equal(context.currentTurn().turn_id, "new");
});

test("snapshotによるターン切替は食事演出を直ちに片付ける", () => {
  const { context, calls } = loadContext();
  context.handle(state("food-1"));
  context.handle({ type: "food_action", image_url: "/food/1.webp", consume_at_ms: 1000, duration_ms: 3000 });
  context.handle(segment("food-1"));

  context.handle({ type: "snapshot", current: null, history: [] });

  assert.equal(calls.filter((call) => call === "clearFood").length, 1);
  assert.equal(context.currentTurn(), undefined);
});
