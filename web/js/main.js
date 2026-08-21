import { AudioQueue } from "./audio-queue.js";
import { ConversationHistory } from "./history.js";
import { isEmotion } from "./motion.js";
import { createSourceButton, SourceDialog } from "./sources.js";
import { VrmViewer } from "./vrm-viewer.js?v=2";

const elements = {
  startScreen: document.querySelector("#start-screen"),
  start: document.querySelector("#start"),
  startError: document.querySelector("#start-error"),
  canvas: document.querySelector("#vrm-canvas"),
  viewerMessage: document.querySelector("#viewer-message"),
  panel: document.querySelector("#panel"),
  loader: document.querySelector("#answer-loader"),
  answer: document.querySelector("#answer"),
  answerText: document.querySelector("#answer-text"),
  notice: document.querySelector("#submission-status"),
  history: document.querySelector("#conversation-history"),
  historyList: document.querySelector("#history-list"),
  sourceDialog: document.querySelector("#source-dialog"),
  sourceList: document.querySelector("#source-list"),
  sourceClose: document.querySelector("#source-close"),
};

const sourceDialog = new SourceDialog(elements.sourceDialog, elements.sourceList, elements.sourceClose);
const historyView = new ConversationHistory(
  elements.history,
  elements.historyList,
  (sources) => sourceDialog.open(sources),
);

let viewer;
let queue;
let socket;
let started = false;
let currentTurn;
let motionPlayedForTurn;
let reconnectTimer;
const receivedTurns = new Set();
let currentSourceButton;

function clearAnswer() {
  elements.answerText.textContent = "";
  currentSourceButton?.remove();
  currentSourceButton = undefined;
}

function showCurrentSources(sources) {
  currentSourceButton?.remove();
  currentSourceButton = createSourceButton(sources, (links) => sourceDialog.open(links));
  if (currentSourceButton) elements.answer.append(currentSourceButton);
}

function showError(message) {
  elements.notice.textContent = message;
  elements.notice.dataset.kind = "error";
}

function showViewerMessage(message = "") {
  elements.viewerMessage.textContent = message;
  elements.viewerMessage.hidden = !message;
}

function setTurn(turn) {
  const isNewTurn = turn?.turn_id !== currentTurn?.turn_id;
  currentTurn = turn;
  if (!turn) {
    clearAnswer();
    elements.answer.hidden = true;
    elements.loader.hidden = true;
    elements.panel.hidden = true;
    return;
  }

  if (isNewTurn) {
    clearAnswer();
  }
  const hasAnswer = Boolean(elements.answerText.textContent);
  elements.answer.hidden = !hasAnswer;
  elements.loader.hidden = hasAnswer;
  elements.panel.hidden = false;
}

function setEmotion(value) {
  viewer?.setEmotion(isEmotion(value) ? value : "neutral");
}

function cleanTurn(turnId) {
  receivedTurns.delete(turnId);
  if (currentTurn?.turn_id !== turnId) return;
  setEmotion("neutral");
  viewer?.resumeIdle();
  setTurn(undefined);
}

function connect() {
  if (!started) return;
  const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(`${scheme}//${window.location.host}/ws`);
  socket.addEventListener("message", (event) => {
    try {
      handleServerEvent(JSON.parse(event.data));
    } catch (error) {
      console.error("表示イベントを処理できませんでした", error);
      showError("表示の更新に失敗しました。ページを再読み込みしてください。");
    }
  });
  socket.addEventListener("close", () => {
    if (!started) return;
    window.clearTimeout(reconnectTimer);
    reconnectTimer = window.setTimeout(connect, 2000);
  });
  socket.addEventListener("error", () => socket.close());
}

function handleServerEvent(event) {
  switch (event.type) {
    case "snapshot":
      historyView.render(event.history || []);
      if (currentTurn && currentTurn.turn_id !== event.current?.turn_id) {
        queue.cancelTurn(currentTurn.turn_id);
        receivedTurns.delete(currentTurn.turn_id);
        viewer.stopLipSync();
        setEmotion("neutral");
      }
      if (event.current) {
        setTurn(event.current);
      } else {
        setTurn(undefined);
      }
      break;
    case "history":
      historyView.render(event.turns || []);
      break;
    case "state":
      if (currentTurn?.turn_id !== event.turn.turn_id) {
        if (currentTurn) {
          queue.cancelTurn(currentTurn.turn_id);
          receivedTurns.delete(currentTurn.turn_id);
          viewer.stopLipSync();
          setEmotion("neutral");
        }
        clearAnswer();
        motionPlayedForTurn = undefined;
      }
      setTurn(event.turn);
      break;
    case "segment":
      receiveSegment(event);
      break;
    case "complete":
      if (!receivedTurns.has(event.turn_id)) {
        cleanTurn(event.turn_id);
      }
      break;
    case "cancelled":
      queue.cancelTurn(event.turn_id);
      receivedTurns.delete(event.turn_id);
      cleanTurn(event.turn_id);
      break;
    case "error":
      queue.cancelTurn(event.turn_id);
      receivedTurns.delete(event.turn_id);
      if (currentTurn?.turn_id === event.turn_id) {
        showError(event.message || "回答の生成に失敗しました。");
        cleanTurn(event.turn_id);
      }
      break;
    case "idle":
      break;
    default:
      console.warn("未対応の表示イベントです", event);
  }
}

function receiveSegment(segment) {
  receivedTurns.add(segment.turn_id);
  if (currentTurn?.turn_id !== segment.turn_id) {
    if (currentTurn) {
      queue.cancelTurn(currentTurn.turn_id);
      receivedTurns.delete(currentTurn.turn_id);
    }
    viewer.stopLipSync();
    setEmotion("neutral");
    setTurn({ turn_id: segment.turn_id, question: "" });
    clearAnswer();
    motionPlayedForTurn = undefined;
  }
  const isFiller = segment.kind === "filler";
  if (!isFiller) {
    elements.loader.hidden = true;
    elements.answer.hidden = false;
    elements.answerText.textContent += segment.text;
    if (segment.is_last) showCurrentSources(segment.sources);
  }
  queue.enqueue({
    url: segment.audio_url,
    turnId: segment.turn_id,
    sequence: segment.sequence,
    durationMs: segment.duration_ms,
    meta: segment,
  });
}

function onAudioStart(item, analyser) {
  const segment = item.meta;
  setEmotion(segment.kind === "filler" ? "neutral" : segment.emotion);
  viewer.startLipSync(analyser);
  if (segment.kind !== "filler" && motionPlayedForTurn !== segment.turn_id && segment.motion && isEmotion(segment.emotion)) {
    motionPlayedForTurn = segment.turn_id;
    viewer.playEmotionMotion(segment.emotion);
  }
}

function onAudioEnd(item) {
  viewer.stopLipSync();
  if (item.meta.is_last) cleanTurn(item.turnId);
}

async function startMain() {
  elements.start.disabled = true;
  elements.startError.textContent = "";
  try {
    queue = new AudioQueue({
      onStart: onAudioStart,
      onEnd: onAudioEnd,
      onError: () => showError("音声を再生できませんでした。"),
    });
    await queue.unlock();

    const response = await fetch("/api/display-config", { cache: "no-store" });
    if (!response.ok) throw new Error(`表示設定を取得できませんでした (${response.status})`);
    const config = await response.json();

    viewer = new VrmViewer(elements.canvas, showViewerMessage);
    await viewer.load(config);
    started = true;
    elements.startScreen.hidden = true;
    connect();
  } catch (error) {
    console.error(error);
    elements.startError.textContent = error.message || "表示を開始できませんでした。";
    elements.start.disabled = false;
    queue?.dispose();
    queue = undefined;
    viewer?.dispose();
    viewer = undefined;
  }
}

elements.start.addEventListener("click", startMain);
window.addEventListener("beforeunload", () => {
  window.clearTimeout(reconnectTimer);
  queue?.dispose();
  viewer?.dispose();
  socket?.close();
});
