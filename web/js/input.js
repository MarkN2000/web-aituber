const form = document.querySelector("#submission-form");
const text = document.querySelector("#text");
const image = document.querySelector("#image");
const submitButton = document.querySelector("#submit-button");
const status = document.querySelector("#submission-status");
const textCount = document.querySelector("#text-count");

function setStatus(message, kind = "") {
  status.textContent = message;
  status.dataset.kind = kind;
}

function updateTextCount() {
  textCount.textContent = String(text.value.length);
}

text.addEventListener("input", updateTextCount);

form.addEventListener("submit", async (event) => {
  event.preventDefault();

  if (!text.value.trim()) {
    setStatus("質問を入力してください。", "error");
    text.focus();
    return;
  }

  if (image.files.length > 1) {
    setStatus("画像は1枚だけ選択してください。", "error");
    return;
  }
  if (image.files[0]?.size > 10 * 1024 * 1024) {
    setStatus("画像は10MB以下にしてください。", "error");
    return;
  }

  submitButton.disabled = true;
  setStatus("質問を送信しています…");

  try {
    const response = await fetch("/api/submissions", {
      method: "POST",
      body: new FormData(form),
    });

    if (!response.ok) {
      const body = await response.json().catch(() => ({}));
      throw new Error(body.error || "送信を受け付けられませんでした。");
    }

    form.reset();
    updateTextCount();
    setStatus("受け付けました。順番にお答えします。", "success");
  } catch (error) {
    console.error(error);
    setStatus(error.message || "送信に失敗しました。通信を確認して、もう一度お試しください。", "error");
  } finally {
    submitButton.disabled = false;
  }
});

updateTextCount();
