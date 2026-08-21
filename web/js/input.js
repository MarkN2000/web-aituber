const form = document.querySelector("#submission-form");
const text = document.querySelector("#text");
const submitButton = document.querySelector("#submit-button");
const status = document.querySelector("#submission-status");
const textCount = document.querySelector("#text-count");

let isSubmitting = false;

function setStatus(message, kind = "") {
  status.textContent = message;
  status.dataset.kind = kind;
}

function updateSubmitState() {
  submitButton.disabled = isSubmitting || !text.value.trim();
  submitButton.dataset.loading = String(isSubmitting);
  if (submitButton.classList.contains("composer-send-button")) {
    const label = isSubmitting ? "質問を送信中" : "質問を送る";
    submitButton.setAttribute("aria-label", label);
    submitButton.title = label;
  }
}

function updateTextCount() {
  textCount.textContent = String(text.value.length);
  updateSubmitState();
}

text.addEventListener("input", updateTextCount);

form.addEventListener("submit", async (event) => {
  event.preventDefault();

  if (!text.value.trim()) {
    setStatus("質問を入力してください。", "error");
    text.focus();
    return;
  }

  isSubmitting = true;
  updateSubmitState();
  setStatus("");

  try {
    const formData = new FormData(form);
    const response = await fetch("/api/submissions", {
      method: "POST",
      body: formData,
    });

    if (!response.ok) {
      const body = await response.json().catch(() => ({}));
      throw new Error(body.error || "送信を受け付けられませんでした。");
    }

    form.reset();
    updateTextCount();
    setStatus("");
  } catch (error) {
    console.error(error);
    setStatus(error.message || "送信に失敗しました。通信を確認して、もう一度お試しください。", "error");
  } finally {
    isSubmitting = false;
    updateSubmitState();
  }
});

updateTextCount();
