import { AudioQueue } from "./audio-queue.js";
import { isEmotion } from "./motion.js";
import { VrmViewer } from "./vrm-viewer.js";

const elements = {
  startScreen: document.querySelector("#start-screen"),
  start: document.querySelector("#start"),
  startError: document.querySelector("#start-error"),
  canvas: document.querySelector("#vrm-canvas"),
  viewerMessage: document.querySelector("#viewer-message"),
  status: document.querySelector("#status"),
  question: document.querySelector("#question"),
  answer: document.querySelector("#answer"),
};

let viewer;
let queue;
let socket;
let started = false;
let currentTurn;
let motionPlayedForTurn;
let reconnectTimer;
const receivedTurns = new Set();

function setStatus(message) {
  elements.status.textContent = message;
}

function showViewerMessage(message = "") {
  elements.viewerMessage.textContent = message;
  elements.viewerMessage.hidden = !message;
}

function setTurn(turn) {
  currentTurn = turn;
  elements.question.textContent = turn ? `質問: ${turn.question}` : "";
  if (!turn) {
    elements.answer.textContent = "";
  }
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
  setStatus("次の質問を待っています");
}

function connect() {
  if (!started) return;
  const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(`${scheme}//${window.location.host}/ws`);
  setStatus("サーバーへ接続中です");

  socket.addEventListener("open", () => {
    setStatus(currentTurn ? "回答を受信中です" : "次の質問を待っています");
  });
  socket.addEventListener("message", (event) => {
    try {
      handleServerEvent(JSON.parse(event.data));
    } catch (error) {
      console.error("表示イベントを処理できませんでした", error);
      setStatus("表示イベントの処理に失敗しました");
    }
  });
  socket.addEventListener("close", () => {
    if (!started) return;
    setStatus("接続が切れました。再接続しています");
    window.clearTimeout(reconnectTimer);
    reconnectTimer = window.setTimeout(connect, 2000);
  });
  socket.addEventListener("error", () => socket.close());
}

function handleServerEvent(event) {
  switch (event.type) {
    case "snapshot":
      if (currentTurn && currentTurn.turn_id !== event.current?.turn_id) {
        queue.cancelTurn(currentTurn.turn_id);
        receivedTurns.delete(currentTurn.turn_id);
        viewer.stopLipSync();
        setEmotion("neutral");
      }
      if (event.current) {
        setTurn(event.current);
        setStatus(event.current.status === "generating" ? "回答を生成中です" : "回答を受信中です");
      } else {
        setTurn(undefined);
        setStatus("次の質問を待っています");
      }
      break;
    case "state":
      if (currentTurn?.turn_id !== event.turn.turn_id) {
        if (currentTurn) {
          queue.cancelTurn(currentTurn.turn_id);
          receivedTurns.delete(currentTurn.turn_id);
          viewer.stopLipSync();
          setEmotion("neutral");
        }
        elements.answer.textContent = "";
        motionPlayedForTurn = undefined;
      }
      setTurn(event.turn);
      setStatus(event.turn.status === "generating" ? "回答を生成中です" : "回答を受信中です");
      break;
    case "segment":
      receiveSegment(event);
      break;
    case "complete":
      if (receivedTurns.has(event.turn_id)) {
        setStatus("回答を再生中です");
      } else {
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
        setStatus(`回答を続けられませんでした: ${event.message}`);
        setEmotion("neutral");
      }
      break;
    case "idle":
      if (!currentTurn) setStatus("次の質問を待っています");
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
    elements.answer.textContent = "";
    motionPlayedForTurn = undefined;
  }
  elements.answer.textContent += segment.text;
  setStatus("回答を再生中です");
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
  setEmotion(segment.emotion);
  viewer.startLipSync(analyser);
  if (motionPlayedForTurn !== segment.turn_id && segment.motion && isEmotion(segment.emotion)) {
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
      onError: () => setStatus("音声を再生できませんでした"),
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

