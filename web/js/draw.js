const CANVAS_SIZE = 512;
const VRM_IMAGE_SIZE = 256;
const AI_IMAGE_SIZE = 128;
const WEBP_QUALITY = 0.75;
const ALPHA_THRESHOLD = 128;
const GAP_CLOSE_RADIUS = 2;
const UNDO_HISTORY_LIMIT = 10;
const COLOR_PICKER_SIZE = 256;
const COLOR_PICKER_CENTER = COLOR_PICKER_SIZE / 2;
const HUE_RING_OUTER_RADIUS = 124;
const HUE_RING_INNER_RADIUS = 96;
const HUE_RING_RADIUS = (HUE_RING_OUTER_RADIUS + HUE_RING_INNER_RADIUS) / 2;
const SV_FIELD_SIZE = 124;
const SV_FIELD_START = (COLOR_PICKER_SIZE - SV_FIELD_SIZE) / 2;

const canvas = document.querySelector("#food-canvas");
const context = canvas.getContext("2d");
const colorPicker = document.querySelector("#color-picker");
const colorPickerContext = colorPicker.getContext("2d");
const colorButtons = [...document.querySelectorAll("[data-draw-color]")];
const toolButtons = [...document.querySelectorAll("[data-draw-tool]")];
const brushSizeButtons = [...document.querySelectorAll("[data-brush-size]")];
const undoButton = document.querySelector("#undo-canvas");
const clearButton = document.querySelector("#clear-canvas");
const submitButton = document.querySelector("#submit-food");
const status = document.querySelector("#draw-status");
const drawCursor = document.querySelector("#draw-cursor");
const drawingSurface = document.querySelector(".drawing-surface");

let activePointer;
let previousPoint;
let activeTool = "pen";
let brushSize = 20;
let selectedColor = "#e85d3f";
let selectedHue = 10;
let selectedSaturation = 0.76;
let selectedValue = 0.91;
let colorPointer;
let colorPickerRegion;
let hasDrawing = false;
let isSubmitting = false;
const undoHistory = [];

function clamp01(value) {
  return Math.min(Math.max(value, 0), 1);
}

function hsvToRgb(hue, saturation, value) {
  const chroma = value * saturation;
  const segment = ((hue % 360) + 360) % 360 / 60;
  const intermediate = chroma * (1 - Math.abs((segment % 2) - 1));
  const base = segment < 1 ? [chroma, intermediate, 0]
    : segment < 2 ? [intermediate, chroma, 0]
      : segment < 3 ? [0, chroma, intermediate]
        : segment < 4 ? [0, intermediate, chroma]
          : segment < 5 ? [intermediate, 0, chroma]
            : [chroma, 0, intermediate];
  const match = value - chroma;
  return base.map((channel) => Math.round((channel + match) * 255));
}

function rgbToHsv(red, green, blue) {
  const channels = [red / 255, green / 255, blue / 255];
  const maximum = Math.max(...channels);
  const minimum = Math.min(...channels);
  const difference = maximum - minimum;
  let hue = 0;
  if (difference > 0) {
    if (maximum === channels[0]) hue = 60 * (((channels[1] - channels[2]) / difference) % 6);
    else if (maximum === channels[1]) hue = 60 * (((channels[2] - channels[0]) / difference) + 2);
    else hue = 60 * (((channels[0] - channels[1]) / difference) + 4);
  }
  return {
    hue: (hue + 360) % 360,
    saturation: maximum === 0 ? 0 : difference / maximum,
    value: maximum,
  };
}

function hexToRgb(hex) {
  return [
    Number.parseInt(hex.slice(1, 3), 16),
    Number.parseInt(hex.slice(3, 5), 16),
    Number.parseInt(hex.slice(5, 7), 16),
  ];
}

function rgbToHex(red, green, blue) {
  return `#${[red, green, blue].map((channel) => channel.toString(16).padStart(2, "0")).join("")}`;
}

function drawPickerHandle(x, y) {
  colorPickerContext.beginPath();
  colorPickerContext.arc(x, y, 7, 0, Math.PI * 2);
  colorPickerContext.lineWidth = 4;
  colorPickerContext.strokeStyle = "#fff";
  colorPickerContext.stroke();
  colorPickerContext.lineWidth = 1;
  colorPickerContext.strokeStyle = "#202020";
  colorPickerContext.stroke();
}

function renderColorPicker() {
  colorPickerContext.clearRect(0, 0, COLOR_PICKER_SIZE, COLOR_PICKER_SIZE);
  colorPickerContext.save();
  colorPickerContext.translate(COLOR_PICKER_CENTER, COLOR_PICKER_CENTER);
  colorPickerContext.lineWidth = HUE_RING_OUTER_RADIUS - HUE_RING_INNER_RADIUS;
  for (let degree = 0; degree < 360; degree += 1) {
    const start = degree * Math.PI / 180;
    const end = (degree + 1.5) * Math.PI / 180;
    colorPickerContext.beginPath();
    colorPickerContext.arc(0, 0, HUE_RING_RADIUS, start, end);
    colorPickerContext.strokeStyle = `hsl(${degree} 100% 50%)`;
    colorPickerContext.stroke();
  }
  colorPickerContext.restore();

  const field = colorPickerContext.createImageData(SV_FIELD_SIZE, SV_FIELD_SIZE);
  for (let y = 0; y < SV_FIELD_SIZE; y += 1) {
    const value = 1 - y / (SV_FIELD_SIZE - 1);
    for (let x = 0; x < SV_FIELD_SIZE; x += 1) {
      const saturation = x / (SV_FIELD_SIZE - 1);
      const [red, green, blue] = hsvToRgb(selectedHue, saturation, value);
      const offset = (y * SV_FIELD_SIZE + x) * 4;
      field.data[offset] = red;
      field.data[offset + 1] = green;
      field.data[offset + 2] = blue;
      field.data[offset + 3] = 255;
    }
  }
  colorPickerContext.putImageData(field, SV_FIELD_START, SV_FIELD_START);
  colorPickerContext.strokeStyle = "#fff";
  colorPickerContext.lineWidth = 2;
  colorPickerContext.strokeRect(SV_FIELD_START, SV_FIELD_START, SV_FIELD_SIZE, SV_FIELD_SIZE);

  const angle = selectedHue * Math.PI / 180;
  drawPickerHandle(
    COLOR_PICKER_CENTER + Math.cos(angle) * HUE_RING_RADIUS,
    COLOR_PICKER_CENTER + Math.sin(angle) * HUE_RING_RADIUS,
  );
  drawPickerHandle(
    SV_FIELD_START + selectedSaturation * (SV_FIELD_SIZE - 1),
    SV_FIELD_START + (1 - selectedValue) * (SV_FIELD_SIZE - 1),
  );
}

function updateSelectedColor() {
  selectedColor = rgbToHex(...hsvToRgb(selectedHue, selectedSaturation, selectedValue));
  for (const button of colorButtons) {
    button.setAttribute("aria-pressed", String(button.dataset.drawColor.toLowerCase() === selectedColor));
  }
  colorPicker.setAttribute("aria-valuetext", selectedColor);
  renderColorPicker();
}

function selectHexColor(hex) {
  const hsv = rgbToHsv(...hexToRgb(hex));
  selectedHue = hsv.hue;
  selectedSaturation = hsv.saturation;
  selectedValue = hsv.value;
  updateSelectedColor();
}

function colorPickerPoint(event) {
  const bounds = colorPicker.getBoundingClientRect();
  return {
    x: (event.clientX - bounds.left) * (COLOR_PICKER_SIZE / bounds.width),
    y: (event.clientY - bounds.top) * (COLOR_PICKER_SIZE / bounds.height),
  };
}

function pickerRegionAt(point) {
  const offsetX = point.x - COLOR_PICKER_CENTER;
  const offsetY = point.y - COLOR_PICKER_CENTER;
  const distance = Math.hypot(offsetX, offsetY);
  if (distance >= HUE_RING_INNER_RADIUS && distance <= HUE_RING_OUTER_RADIUS) return "hue";
  if (
    point.x >= SV_FIELD_START
    && point.x <= SV_FIELD_START + SV_FIELD_SIZE - 1
    && point.y >= SV_FIELD_START
    && point.y <= SV_FIELD_START + SV_FIELD_SIZE - 1
  ) return "sv";
  return undefined;
}

function updateColorPickerFromPoint(point) {
  if (colorPickerRegion === "hue") {
    selectedHue = (Math.atan2(
      point.y - COLOR_PICKER_CENTER,
      point.x - COLOR_PICKER_CENTER,
    ) * 180 / Math.PI + 360) % 360;
  } else if (colorPickerRegion === "sv") {
    selectedSaturation = clamp01((point.x - SV_FIELD_START) / (SV_FIELD_SIZE - 1));
    selectedValue = 1 - clamp01((point.y - SV_FIELD_START) / (SV_FIELD_SIZE - 1));
  }
  updateSelectedColor();
}

function startColorPicking(event) {
  if (colorPointer !== undefined || event.button > 0) return;
  const point = colorPickerPoint(event);
  colorPickerRegion = pickerRegionAt(point);
  if (!colorPickerRegion) return;
  event.preventDefault();
  colorPointer = event.pointerId;
  colorPicker.setPointerCapture(event.pointerId);
  updateColorPickerFromPoint(point);
}

function continueColorPicking(event) {
  if (event.pointerId !== colorPointer) return;
  event.preventDefault();
  updateColorPickerFromPoint(colorPickerPoint(event));
}

function stopColorPicking(event) {
  if (event.pointerId !== colorPointer) return;
  if (colorPicker.hasPointerCapture(event.pointerId)) colorPicker.releasePointerCapture(event.pointerId);
  colorPointer = undefined;
  colorPickerRegion = undefined;
}

function handleColorPickerKey(event) {
  const step = 2;
  if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
    selectedHue = (selectedHue + (event.key === "ArrowLeft" ? -step : step) + 360) % 360;
  } else if (event.key === "ArrowUp" || event.key === "ArrowDown") {
    const direction = event.key === "ArrowUp" ? 0.02 : -0.02;
    if (event.shiftKey) selectedSaturation = clamp01(selectedSaturation + direction);
    else selectedValue = clamp01(selectedValue + direction);
  } else {
    return;
  }
  event.preventDefault();
  updateSelectedColor();
}

function addUndoState(history, state, limit) {
  history.push(state);
  if (history.length > limit) history.shift();
}

function updateUndoState() {
  undoButton.disabled = isSubmitting || undoHistory.length === 0;
  clearButton.disabled = isSubmitting || !hasDrawing;
}

function rememberCanvas() {
  addUndoState(undoHistory, {
    image: context.getImageData(0, 0, CANVAS_SIZE, CANVAS_SIZE),
    hasDrawing,
  }, UNDO_HISTORY_LIMIT);
  updateUndoState();
}

function discardUndoHistory() {
  undoHistory.length = 0;
  updateUndoState();
}

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
  context.lineWidth = brushSize;
  context.globalCompositeOperation = activeTool === "eraser" ? "destination-out" : "source-over";
  context.strokeStyle = selectedColor;
  context.stroke();
  context.restore();
  hasDrawing = true;
  updateSubmitState();
}

function startDrawing(event) {
  if (activePointer !== undefined || event.button > 0) return;
  event.preventDefault();
  const point = canvasPoint(event);
  if (activeTool === "bucket") {
    fillAt(point);
    return;
  }

  rememberCanvas();
  activePointer = event.pointerId;
  previousPoint = point;
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

function setTool(tool) {
  activeTool = tool;
  for (const button of toolButtons) {
    button.setAttribute("aria-pressed", String(button.dataset.drawTool === tool));
  }
  for (const button of brushSizeButtons) button.disabled = tool === "bucket";
  canvas.dataset.tool = tool;
  drawCursor.dataset.tool = tool;
  updateDrawCursorSize();
}

function setBrushSize(size) {
  brushSize = Number(size);
  for (const button of brushSizeButtons) {
    button.setAttribute("aria-pressed", String(Number(button.dataset.brushSize) === brushSize));
  }
  updateDrawCursorSize();
}

function updateDrawCursorSize() {
  if (activeTool === "bucket") return;
  const bounds = canvas.getBoundingClientRect();
  const displayedSize = Math.max(brushSize * (bounds.width / canvas.width), 4);
  drawCursor.style.setProperty("--cursor-size", `${displayedSize}px`);
}

function moveDrawCursor(event) {
  if (event.pointerType === "touch") {
    drawCursor.hidden = true;
    return;
  }
  const bounds = drawingSurface.getBoundingClientRect();
  drawCursor.style.left = `${event.clientX - bounds.left}px`;
  drawCursor.style.top = `${event.clientY - bounds.top}px`;
  updateDrawCursorSize();
  drawCursor.hidden = false;
}

function hideDrawCursor(event) {
  if (event.pointerId === activePointer) return;
  drawCursor.hidden = true;
}

function rgbaFromHex(hex) {
  return [
    Number.parseInt(hex.slice(1, 3), 16),
    Number.parseInt(hex.slice(3, 5), 16),
    Number.parseInt(hex.slice(5, 7), 16),
    255,
  ];
}

function expandFill(image, width, height, filled, fillColor) {
  const border = new Uint8Array(filled.length);
  for (let index = 0; index < filled.length; index += 1) {
    if (!filled[index]) continue;
    const x = index % width;
    const y = Math.floor(index / width);
    for (let offsetY = -1; offsetY <= 1; offsetY += 1) {
      const targetY = y + offsetY;
      if (targetY < 0 || targetY >= height) continue;
      for (let offsetX = -1; offsetX <= 1; offsetX += 1) {
        const targetX = x + offsetX;
        if (targetX < 0 || targetX >= width) continue;
        const targetIndex = targetY * width + targetX;
        if (!filled[targetIndex]) border[targetIndex] = 1;
      }
    }
  }

  for (let index = 0; index < border.length; index += 1) {
    if (!border[index]) continue;
    const offset = index * 4;
    for (let channel = 0; channel < 4; channel += 1) {
      image.data[offset + channel] = fillColor[channel];
    }
  }
}

function floodFill(image, width, height, startX, startY, fillColor) {
  if (startX < 0 || startX >= width || startY < 0 || startY >= height) return false;

  const startIndex = startY * width + startX;
  const startOffset = startIndex * 4;
  const targetColor = Array.from(image.data.slice(startOffset, startOffset + 4));
  if (targetColor.every((value, index) => value === fillColor[index])) return false;

  const filled = new Uint8Array(width * height);
  const matchesTarget = (index) => {
    const offset = index * 4;
    return targetColor.every((value, channel) => image.data[offset + channel] === value);
  };
  const paint = (index) => {
    const offset = index * 4;
    for (let channel = 0; channel < 4; channel += 1) {
      image.data[offset + channel] = fillColor[channel];
    }
    filled[index] = 1;
  };

  const pending = [startIndex];
  paint(startIndex);
  const visit = (index) => {
    if (!matchesTarget(index)) return;
    paint(index);
    pending.push(index);
  };
  while (pending.length > 0) {
    const index = pending.pop();
    const x = index % width;
    if (x > 0) visit(index - 1);
    if (x + 1 < width) visit(index + 1);
    if (index >= width) visit(index - width);
    if (index + width < width * height) visit(index + width);
  }
  expandFill(image, width, height, filled, fillColor);
  return true;
}

function fillAt(point) {
  const image = context.getImageData(0, 0, CANVAS_SIZE, CANVAS_SIZE);
  const previousImage = context.createImageData(CANVAS_SIZE, CANVAS_SIZE);
  previousImage.data.set(image.data);
  const changed = floodFill(
    image,
    CANVAS_SIZE,
    CANVAS_SIZE,
    Math.floor(point.x),
    Math.floor(point.y),
    rgbaFromHex(selectedColor),
  );
  if (!changed) return;
  addUndoState(undoHistory, { image: previousImage, hasDrawing }, UNDO_HISTORY_LIMIT);
  context.putImageData(image, 0, 0);
  hasDrawing = true;
  updateSubmitState();
}

function undoCanvas() {
  const previous = undoHistory.pop();
  if (!previous) return;
  context.putImageData(previous.image, 0, 0);
  hasDrawing = previous.hasDrawing;
  setStatus("");
  updateSubmitState();
}

function setStatus(message, kind = "") {
  status.textContent = message;
  status.dataset.kind = kind;
}

function updateSubmitState() {
  submitButton.disabled = isSubmitting || !hasDrawing;
  submitButton.textContent = isSubmitting ? "送信中…" : "キャラクターに食べてもらう";
  updateUndoState();
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

    discardUndoHistory();
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
canvas.addEventListener("pointerenter", moveDrawCursor);
canvas.addEventListener("pointermove", moveDrawCursor);
canvas.addEventListener("pointerleave", hideDrawCursor);
colorPicker.addEventListener("pointerdown", startColorPicking);
colorPicker.addEventListener("pointermove", continueColorPicking);
colorPicker.addEventListener("pointerup", stopColorPicking);
colorPicker.addEventListener("pointercancel", stopColorPicking);
colorPicker.addEventListener("keydown", handleColorPickerKey);
for (const button of colorButtons) {
  button.addEventListener("click", () => selectHexColor(button.dataset.drawColor));
}
for (const button of toolButtons) {
  button.addEventListener("click", () => setTool(button.dataset.drawTool));
}
for (const button of brushSizeButtons) {
  button.addEventListener("click", () => setBrushSize(button.dataset.brushSize));
}
undoButton.addEventListener("click", undoCanvas);
clearButton.addEventListener("click", () => {
  if (hasDrawing) {
    rememberCanvas();
    clearCanvas();
  }
  setStatus("");
});
submitButton.addEventListener("click", submitFood);

selectHexColor(selectedColor);
setTool("pen");
setBrushSize(brushSize);
clearCanvas();
