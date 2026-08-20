const token = new URLSearchParams(window.location.search).get("token");

const elements = {
  status: document.querySelector("#admin-status"),
  question: document.querySelector("#admin-question"),
  answer: document.querySelector("#admin-answer"),
  skip: document.querySelector("#skip"),
  error: document.querySelector("#admin-error"),
};

let currentTurn;
let socket;
let reconnectTimer;

function setCurrentTurn(turn) {
  currentTurn = turn;
  elements.question.textContent = turn?.question || "—";
  elements.skip.disabled = !turn;
}

function setStatus(message) {
  elements.status.textContent = message;
}

function connect() {
  const scheme = window.location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(`${scheme}//${window.location.host}/ws`);
  setStatus("サーバーへ接続中です");

  socket.addEventListener("open", () => {
    setStatus(currentTurn ? "処理中" : "待機中");
  });
  socket.addEventListener("message", (message) => {
    try {
      handleServerEvent(JSON.parse(message.data));
    } catch (error) {
      console.error("管理画面のイベントを処理できませんでした", error);
      elements.error.textContent = "状態を更新できませんでした。";
    }
  });
  socket.addEventListener("close", () => {
    setStatus("再接続中");
    elements.skip.disabled = true;
    window.clearTimeout(reconnectTimer);
    reconnectTimer = window.setTimeout(connect, 2000);
  });
  socket.addEventListener("error", () => socket.close());
}

function handleServerEvent(event) {
  switch (event.type) {
    case "snapshot":
      elements.answer.textContent = "—";
      if (event.current) {
        setCurrentTurn(event.current);
        setStatus(event.current.status === "generating" ? "回答生成中" : "発話準備中");
      } else {
        setCurrentTurn(undefined);
        setStatus("待機中");
      }
      break;
    case "state":
      if (currentTurn?.turn_id !== event.turn.turn_id) {
        elements.answer.textContent = "—";
      }
      setCurrentTurn(event.turn);
      setStatus(event.turn.status === "generating" ? "回答生成中" : "発話中");
      break;
    case "segment":
      if (currentTurn?.turn_id !== event.turn_id) {
        setCurrentTurn({ turn_id: event.turn_id, question: "" });
        elements.answer.textContent = "";
      }
      if (elements.answer.textContent === "—") elements.answer.textContent = "";
      elements.answer.textContent += event.text;
      setStatus("発話中");
      break;
    case "complete":
      if (currentTurn?.turn_id === event.turn_id) {
        setStatus("回答完了");
        elements.skip.disabled = true;
      }
      break;
    case "cancelled":
      if (currentTurn?.turn_id === event.turn_id) {
        setStatus("中断しました");
        elements.skip.disabled = true;
      }
      break;
    case "error":
      if (currentTurn?.turn_id === event.turn_id) {
        setStatus("エラー");
        elements.error.textContent = event.message;
        elements.skip.disabled = true;
      }
      break;
    case "idle":
      if (!currentTurn) setStatus("待機中");
      break;
    default:
      console.warn("未対応の管理イベントです", event);
  }
}

async function skipCurrentTurn() {
  if (!currentTurn || !token) return;
  elements.skip.disabled = true;
  elements.error.textContent = "";
  setStatus("中断処理中");

  try {
    const response = await fetch(`/api/admin/skip?token=${encodeURIComponent(token)}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ turn_id: currentTurn.turn_id }),
    });
    if (!response.ok) {
      throw new Error(`中断操作に失敗しました (${response.status})`);
    }
  } catch (error) {
    console.error(error);
    elements.error.textContent = error.message || "中断操作に失敗しました。";
    elements.skip.disabled = false;
  }
}

if (!token) {
  elements.error.textContent = "管理用トークンが指定されていません。";
} else {
  connect();
}

elements.skip.addEventListener("click", skipCurrentTurn);
window.addEventListener("beforeunload", () => {
  window.clearTimeout(reconnectTimer);
  socket?.close();
});
