const CANVAS_SIZE = 512;
const VRM_IMAGE_SIZE = 256;
const AI_IMAGE_SIZE = 128;
const WEBP_QUALITY = 0.75;
const ALPHA_THRESHOLD = 128;
const GAP_CLOSE_RADIUS = 2;

const canvas = document.querySelector("#food-canvas");
const context = canvas.getContext("2d");
const color = document.querySelector("#draw-color");
const brushSize = document.querySelector("#brush-size");
const eraser = document.querySelector("#eraser");
const clearButton = document.querySelector("#clear-canvas");
const submitButton = document.querySelector("#submit-food");
const status = document.querySelector("#draw-status");

let activePointer;
let previousPoint;
let erasing = false;
let hasDrawing = false;
let isSubmitting = false;

function clearCanvas() {
  context.clearRect(0, 0, CANVAS_SIZE, CANVAS_SIZE);
  hasDrawing = false;
  updateSubmitState();
}

function canvasPoint(event) {
  const bounds = canvas.getBoundingClientRect();
  return {
    x: (event.clientX - bounds.left) * (canvas.width / bounds.width),
    y: (event.clientY - bounds.top) * (canvas.height / bounds.height),
  };
}

function drawLine(from, to) {
  context.save();
  context.beginPath();
  context.moveTo(from.x, from.y);
  context.lineTo(to.x, to.y);
  context.lineCap = "round";
  context.lineJoin = "round";
  context.lineWidth = Number(brushSize.value);
  context.globalCompositeOperation = erasing ? "destination-out" : "source-over";
  context.strokeStyle = color.value;
  context.stroke();
  context.restore();
  hasDrawing = true;
  updateSubmitState();
}

function startDrawing(event) {
  if (activePointer !== undefined || event.button > 0) return;
  event.preventDefault();
  activePointer = event.pointerId;
  previousPoint = canvasPoint(event);
  canvas.setPointerCapture(event.pointerId);
  drawLine(previousPoint, { x: previousPoint.x + 0.01, y: previousPoint.y });
}

function continueDrawing(event) {
  if (event.pointerId !== activePointer) return;
  event.preventDefault();
  const point = canvasPoint(event);
  drawLine(previousPoint, point);
  previousPoint = point;
}

function stopDrawing(event) {
  if (event.pointerId !== activePointer) return;
  if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
  activePointer = undefined;
  previousPoint = undefined;
}

function setErasing(value) {
  erasing = value;
  eraser.setAttribute("aria-pressed", String(value));
}

function setStatus(message, kind = "") {
  status.textContent = message;
  status.dataset.kind = kind;
}

function updateSubmitState() {
  submitButton.disabled = isSubmitting || !hasDrawing;
  submitButton.textContent = isSubmitting ? "送信中…" : "食べてもらう";
}

function closeGaps(mask, width, height) {
  const expanded = new Uint8Array(mask.length);
  for (let index = 0; index < mask.length; index += 1) {
    if (!mask[index]) continue;
    const x = index % width;
    const y = Math.floor(index / width);
    for (let offsetY = -GAP_CLOSE_RADIUS; offsetY <= GAP_CLOSE_RADIUS; offsetY += 1) {
      const targetY = y + offsetY;
      if (targetY < 0 || targetY >= height) continue;
      for (let offsetX = -GAP_CLOSE_RADIUS; offsetX <= GAP_CLOSE_RADIUS; offsetX += 1) {
        const targetX = x + offsetX;
        if (targetX >= 0 && targetX < width) expanded[targetY * width + targetX] = 1;
      }
    }
  }

  const closed = new Uint8Array(mask.length);
  for (let index = 0; index < expanded.length; index += 1) {
    if (!expanded[index]) continue;
    const x = index % width;
    const y = Math.floor(index / width);
    let filled = true;
    for (let offsetY = -GAP_CLOSE_RADIUS; offsetY <= GAP_CLOSE_RADIUS && filled; offsetY += 1) {
      const targetY = y + offsetY;
      if (targetY < 0 || targetY >= height) continue;
      for (let offsetX = -GAP_CLOSE_RADIUS; offsetX <= GAP_CLOSE_RADIUS; offsetX += 1) {
        const targetX = x + offsetX;
        if (targetX >= 0 && targetX < width && !expanded[targetY * width + targetX]) {
          filled = false;
          break;
        }
      }
    }
    if (filled) closed[index] = 1;
  }
  return closed;
}

function findExterior(mask, width, height) {
  const exterior = new Uint8Array(mask.length);
  const queue = new Int32Array(mask.length);
  let head = 0;
  let tail = 0;

  function enqueue(index) {
    if (mask[index] || exterior[index]) return;
    exterior[index] = 1;
    queue[tail] = index;
    tail += 1;
  }

  for (let x = 0; x < width; x += 1) {
    enqueue(x);
    enqueue((height - 1) * width + x);
  }
  for (let y = 1; y < height - 1; y += 1) {
    enqueue(y * width);
    enqueue(y * width + width - 1);
  }

  while (head < tail) {
    const index = queue[head];
    head += 1;
    const x = index % width;
    if (x > 0) enqueue(index - 1);
    if (x + 1 < width) enqueue(index + 1);
    if (index >= width) enqueue(index - width);
    if (index + width < mask.length) enqueue(index + width);
  }
  return exterior;
}

function fillEnclosedAreas(image, width, height) {
  const lineMask = new Uint8Array(width * height);
  for (let index = 0; index < lineMask.length; index += 1) {
    lineMask[index] = image.data[index * 4 + 3] >= ALPHA_THRESHOLD ? 1 : 0;
  }

  const exterior = findExterior(closeGaps(lineMask, width, height), width, height);
  for (let index = 0; index < exterior.length; index += 1) {
    const offset = index * 4;
    if (image.data[offset + 3] >= ALPHA_THRESHOLD || exterior[index]) continue;
    image.data[offset] = 255;
    image.data[offset + 1] = 255;
    image.data[offset + 2] = 255;
    image.data[offset + 3] = 255;
  }
}

function createFilledCanvas() {
  const filled = document.createElement("canvas");
  filled.width = CANVAS_SIZE;
  filled.height = CANVAS_SIZE;
  const filledContext = filled.getContext("2d");
  filledContext.drawImage(canvas, 0, 0);

  const image = filledContext.getImageData(0, 0, CANVAS_SIZE, CANVAS_SIZE);
  fillEnclosedAreas(image, CANVAS_SIZE, CANVAS_SIZE);
  filledContext.putImageData(image, 0, 0);
  return filled;
}

function createSubmissionCanvases() {
  const filled = createFilledCanvas();

  const vrm = document.createElement("canvas");
  vrm.width = VRM_IMAGE_SIZE;
  vrm.height = VRM_IMAGE_SIZE;
  vrm.getContext("2d").drawImage(filled, 0, 0, VRM_IMAGE_SIZE, VRM_IMAGE_SIZE);

  const ai = document.createElement("canvas");
  ai.width = AI_IMAGE_SIZE;
  ai.height = AI_IMAGE_SIZE;
  const aiContext = ai.getContext("2d");
  aiContext.fillStyle = "#fff";
  aiContext.fillRect(0, 0, AI_IMAGE_SIZE, AI_IMAGE_SIZE);
  aiContext.drawImage(filled, 0, 0, AI_IMAGE_SIZE, AI_IMAGE_SIZE);

  return { vrm, ai };
}

function canvasBlob(canvas) {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error("描いた画像を作成できませんでした。"));
    }, "image/webp", WEBP_QUALITY);
  });
}

async function submitFood() {
  if (!hasDrawing || isSubmitting) return;
  isSubmitting = true;
  updateSubmitState();
  setStatus("");

  try {
    const canvases = createSubmissionCanvases();
    const [vrmImage, aiImage] = await Promise.all([
      canvasBlob(canvases.vrm),
      canvasBlob(canvases.ai),
    ]);
    const formData = new FormData();
    formData.set("vrm_image", vrmImage, "food-vrm.webp");
    formData.set("ai_image", aiImage, "food-ai.webp");
    const response = await fetch("/api/food-submissions", {
      method: "POST",
      body: formData,
    });
    if (!response.ok) {
      const body = await response.json().catch(() => ({}));
      throw new Error(body.error || "送信を受け付けられませんでした。");
    }

    clearCanvas();
    setStatus("送信しました。順番になるとAIキャラクターが食べます。", "success");
  } catch (error) {
    console.error(error);
    setStatus(error.message || "送信に失敗しました。通信を確認して、もう一度お試しください。", "error");
  } finally {
    isSubmitting = false;
    updateSubmitState();
  }
}

canvas.addEventListener("pointerdown", startDrawing);
canvas.addEventListener("pointermove", continueDrawing);
canvas.addEventListener("pointerup", stopDrawing);
canvas.addEventListener("pointercancel", stopDrawing);
color.addEventListener("input", () => setErasing(false));
eraser.addEventListener("click", () => setErasing(!erasing));
clearButton.addEventListener("click", () => {
  clearCanvas();
  setStatus("");
});
submitButton.addEventListener("click", submitFood);

clearCanvas();
