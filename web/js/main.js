import { AudioQueue } from "./audio-queue.js?v=4";
import { BackgroundMusic } from "./background-music.js?v=7";
import { ConversationHistory } from "./history.js?v=9";
import { INVALID_EVENT_MESSAGE, showInvalidEventScreen } from "./invalid-event.js?v=1";
import { isDebugEnabled, renderDebugState } from "./debug.js?v=2";
import { isEmotion } from "./motion.js";
import { createSourceButton, SourceDialog } from "./sources.js";
import { VrmViewer } from "./vrm-viewer.js?v=12";

const debugEnabled = isDebugEnabled(window.location.search);
const eventBasePath = window.location.pathname.match(/^\/event\/[^/]+/)?.[0] || "";

const elements = {
  startScreen: document.querySelector("#start-screen"),
  start: document.querySelector("#start"),
  startError: document.querySelector("#start-error"),
  stage: document.querySelector("#stage"),
  canvas: document.querySelector("#vrm-canvas"),
  viewerMessage: document.querySelector("#viewer-message"),
  debugOverlay: document.querySelector("#debug-overlay"),
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

let debugState = debugEnabled ? { connection: "connecting" } : undefined;
let debugStateKey;

function updateDebugState(partialState) {
  if (!debugState) return;
  debugState = { ...debugState, ...partialState };
  const nextKey = JSON.stringify(debugState);
  if (nextKey === debugStateKey) return;
  debugStateKey = nextKey;
  renderDebugState(elements.debugOverlay, debugState);
}

function resetDebugState() {
  if (!debugEnabled) return;
  debugState = { connection: "connecting" };
  debugStateKey = undefined;
  elements.debugOverlay.hidden = true;
  elements.debugOverlay.textContent = "";
}

const sourceDialog = new SourceDialog(elements.sourceDialog, elements.sourceList, elements.sourceClose);
const historyView = new ConversationHistory(
  elements.history,
  elements.historyList,
  (sources) => sourceDialog.open(sources),
);

let viewer;
let queue;
let backgroundMusic;
let socket;
let started = false;
let currentTurn;
let motionPlayedForTurn;
let reconnectTimer;
let pageVisible = document.visibilityState === "visible";
let displayConfig;
let appliedViewerConfigKey;
let pendingViewerConfig;
let viewerReloading = false;
let eventEnded = false;

function applyBackground(config) {
  elements.stage.style.backgroundColor = config.background_color || "#202632";
  elements.stage.style.backgroundImage = config.background_image_url
    ? `url(${JSON.stringify(config.background_image_url)})`
    : "none";
}

function viewerConfigKey(config) {
  return JSON.stringify({
    vrm_url: config.vrm_url,
    idle_motions: config.idle_motions,
    emotion_motions: Object.fromEntries(
      Object.entries(config.emotion_motions || {}).sort(([left], [right]) => left.localeCompare(right)),
    ),
    food_prop: config.food_prop,
    camera: config.camera,
    light: config.light,
  });
}

async function fetchDisplayConfig() {
  const response = await fetch(`${eventBasePath}/api/display-config`, { cache: "no-store" });
  if (response.status === 404) {
    endEventAccess();
    throw new Error(INVALID_EVENT_MESSAGE);
  }
  if (!response.ok) throw new Error(`表示設定を取得できませんでした (${response.status})`);
  return response.json();
}

async function applyPendingViewerConfig() {
  if (currentTurn || viewerReloading || !pendingViewerConfig) return;
  const config = pendingViewerConfig;
  pendingViewerConfig = undefined;
  viewerReloading = true;
  showViewerMessage("モデルを更新しています。");
  viewer?.dispose();
  const nextViewer = new VrmViewer(elements.canvas, showViewerMessage, {
    showFoodPropGizmo: debugEnabled,
    onDebugStateChange: debugEnabled
      ? updateDebugState
      : undefined,
  });
  viewer = nextViewer;
  try {
    await nextViewer.load(config);
    appliedViewerConfigKey = viewerConfigKey(config);
    showViewerMessage();
  } catch (error) {
    console.error("モデルを更新できませんでした", error);
    nextViewer.dispose();
    if (viewer === nextViewer) viewer = undefined;
    showViewerMessage(error.message || "モデルを更新できませんでした。");
  } finally {
    viewerReloading = false;
    if (!currentTurn && pendingViewerConfig) void applyPendingViewerConfig();
  }
}

function applyUpdatedDisplayConfig(config) {
  const previous = displayConfig;
  displayConfig = config;
  applyBackground(config);
  backgroundMusic?.setLevels(
    config.background_music_volume,
    config.background_music_duck_ratio,
  );
  if (previous?.background_music_url !== config.background_music_url) {
    void backgroundMusic?.play(
      config.background_music_url,
      config.background_music_volume,
      config.background_music_duck_ratio,
    );
  }
  if (viewerConfigKey(config) !== appliedViewerConfigKey) {
    pendingViewerConfig = config;
    void applyPendingViewerConfig();
  }
}

async function refreshDisplayConfig() {
  try {
    applyUpdatedDisplayConfig(await fetchDisplayConfig());
  } catch (error) {
    console.error("表示設定を更新できませんでした", error);
    showError("表示設定を更新できませんでした。");
  }
}
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

function pausePlayback() {
  void queue?.suspend().catch((error) => console.error("TTS音声を一時停止できませんでした", error));
  void backgroundMusic?.pause().catch((error) => console.error("BGMを一時停止できませんでした", error));
}

function resumePlayback() {
  void queue?.resume().catch((error) => {
    console.error("TTS音声を再開できませんでした", error);
    showError("音声を再生できませんでした。");
  });
  void backgroundMusic?.resumePlayback();
}

function handleVisibilityChange() {
  const visible = document.visibilityState === "visible";
  if (pageVisible === visible) return;
  pageVisible = visible;
  viewer?.setRenderingEnabled(visible);
  if (visible) {
    resumePlayback();
  } else {
    pausePlayback();
  }
}

function setTurn(turn) {
  const isNewTurn = turn?.turn_id !== currentTurn?.turn_id;
  currentTurn = turn;
  if (!turn) {
    clearAnswer();
    elements.answer.hidden = true;
    elements.loader.hidden = true;
    elements.panel.hidden = true;
    void applyPendingViewerConfig();
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
  backgroundMusic?.setDucked(false);
  setEmotion("neutral");
  viewer?.clearFoodProp();
  viewer?.resumeIdle();
  setTurn(undefined);
}

function cancelTurnAudio(turnId) {
  queue.cancelTurn(turnId);
  backgroundMusic?.setDucked(false);
}

function connect(connectionState = "connecting") {
  if (!started) return;
  updateDebugState({ connection: connectionState });
  const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(`${scheme}//${window.location.host}${eventBasePath}/ws`);
  socket.addEventListener("open", () => {
    updateDebugState({ connection: "connected" });
    void refreshDisplayConfig();
  });
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
    updateDebugState({ connection: "reconnecting" });
    window.clearTimeout(reconnectTimer);
    reconnectTimer = window.setTimeout(() => connect("reconnecting"), 2000);
  });
  socket.addEventListener("error", () => socket.close());
}

function handleServerEvent(event) {
  switch (event.type) {
    case "snapshot":
      historyView.render(event.history || []);
      if (currentTurn && currentTurn.turn_id !== event.current?.turn_id) {
        cancelTurnAudio(currentTurn.turn_id);
        receivedTurns.delete(currentTurn.turn_id);
        viewer?.stopLipSync();
        viewer?.clearFoodProp();
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
    case "display_config_changed":
      void refreshDisplayConfig();
      break;
    case "event_ended":
      endEventAccess();
      break;
    case "state":
      if (currentTurn?.turn_id !== event.turn.turn_id) {
        if (currentTurn) {
          cancelTurnAudio(currentTurn.turn_id);
          receivedTurns.delete(currentTurn.turn_id);
          viewer?.stopLipSync();
          viewer?.clearFoodProp();
          setEmotion("neutral");
        }
        clearAnswer();
        motionPlayedForTurn = undefined;
        backgroundMusic?.setDucked(false);
      }
      setTurn(event.turn);
      break;
    case "food_action":
      elements.panel.hidden = true;
      viewer?.playFoodAction(event.image_url, event.consume_at_ms, event.duration_ms);
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
      cancelTurnAudio(event.turn_id);
      receivedTurns.delete(event.turn_id);
      cleanTurn(event.turn_id);
      break;
    case "error":
      cancelTurnAudio(event.turn_id);
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

function endEventAccess() {
  eventEnded = true;
  started = false;
  window.clearTimeout(reconnectTimer);
  socket?.close();
  queue?.dispose();
  queue = undefined;
  backgroundMusic?.dispose();
  backgroundMusic = undefined;
  viewer?.dispose();
  viewer = undefined;
  showInvalidEventScreen();
}

function receiveSegment(segment) {
  receivedTurns.add(segment.turn_id);
  if (currentTurn?.turn_id !== segment.turn_id) {
    if (currentTurn) {
      cancelTurnAudio(currentTurn.turn_id);
      receivedTurns.delete(currentTurn.turn_id);
    }
    viewer?.stopLipSync();
    backgroundMusic?.setDucked(false);
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
  backgroundMusic?.setDucked(true);
  setEmotion(segment.kind === "filler" ? "neutral" : segment.emotion);
  viewer?.startLipSync(analyser);
  if (segment.kind !== "filler" && motionPlayedForTurn !== segment.turn_id && segment.motion && isEmotion(segment.emotion)) {
    motionPlayedForTurn = segment.turn_id;
    viewer?.playEmotionMotion(segment.emotion);
  }
}

function onAudioEnd(item) {
  backgroundMusic?.setDucked(false);
  viewer?.stopLipSync();
  if (item.meta.is_last) cleanTurn(item.turnId);
}

async function startMain() {
  resetDebugState();
  elements.start.disabled = true;
  elements.startError.textContent = "";
  let backgroundMusicResume;
  try {
    try {
      backgroundMusic = new BackgroundMusic({
        onError: () => showError("BGMを再生できませんでした。"),
      });
      if (!pageVisible) {
        void backgroundMusic.pause().catch((error) => console.error("BGMを一時停止できませんでした", error));
      }
      backgroundMusicResume = backgroundMusic.resume().catch((error) => {
        console.error("BGMの音声機能を開始できませんでした", error);
        showError("BGMを再生できませんでした。");
      });
    } catch (error) {
      console.error("BGMの音声機能を準備できませんでした", error);
      showError("BGMを再生できませんでした。");
    }
    queue = new AudioQueue({
      onStart: onAudioStart,
      onEnd: onAudioEnd,
      onError: () => {
        showError("音声を再生できませんでした。");
      },
    });
    if (!pageVisible) {
      void queue.suspend().catch((error) => console.error("TTS音声を一時停止できませんでした", error));
    }
    const queueUnlock = queue.unlock();
    await queueUnlock;
    await backgroundMusicResume;

    const config = await fetchDisplayConfig();
    applyBackground(config);
    void backgroundMusic?.play(
      config.background_music_url,
      config.background_music_volume,
      config.background_music_duck_ratio,
    );

    viewer = new VrmViewer(elements.canvas, showViewerMessage, {
      showFoodPropGizmo: debugEnabled,
      onDebugStateChange: debugEnabled
        ? updateDebugState
        : undefined,
    });
    await viewer.load(config);
    displayConfig = config;
    appliedViewerConfigKey = viewerConfigKey(config);
    started = true;
    elements.startScreen.hidden = true;
    connect();
  } catch (error) {
    console.error(error);
    elements.startError.textContent = error.message || "表示を開始できませんでした。";
    elements.start.disabled = eventEnded;
    queue?.dispose();
    queue = undefined;
    backgroundMusic?.dispose();
    backgroundMusic = undefined;
    viewer?.dispose();
    viewer = undefined;
    resetDebugState();
  }
}

elements.start.addEventListener("click", startMain);
document.addEventListener("visibilitychange", handleVisibilityChange);
window.addEventListener("beforeunload", () => {
  window.clearTimeout(reconnectTimer);
  queue?.dispose();
  backgroundMusic?.dispose();
  viewer?.dispose();
  socket?.close();
});
