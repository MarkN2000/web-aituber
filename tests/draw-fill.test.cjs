const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const drawSource = fs.readFileSync(path.join(__dirname, "../web/js/draw.js"), "utf8");
const undoHistorySource = drawSource.slice(
  drawSource.indexOf("function addUndoState"),
  drawSource.indexOf("function updateUndoState"),
);
const bucketSource = drawSource.slice(
  drawSource.indexOf("function rgbaFromHex"),
  drawSource.indexOf("function fillAt"),
);
const algorithmSource = drawSource.slice(
  drawSource.indexOf("function closeGaps"),
  drawSource.indexOf("function createFilledCanvas"),
);
const submissionSource = drawSource.slice(
  drawSource.indexOf("function createFilledCanvas"),
  drawSource.indexOf("function canvasBlob"),
);
const context = vm.createContext({});
vm.runInContext(
  `const ALPHA_THRESHOLD = 128; const GAP_CLOSE_RADIUS = 2; ${algorithmSource}; this.fill = fillEnclosedAreas;`,
  context,
);
const bucketContext = vm.createContext({});
vm.runInContext(
  `${bucketSource}; this.fill = floodFill; this.color = rgbaFromHex;`,
  bucketContext,
);
const undoHistoryContext = vm.createContext({});
vm.runInContext(`${undoHistorySource}; this.add = addUndoState;`, undoHistoryContext);

function createImage(width, height) {
  return { data: new Uint8ClampedArray(width * height * 4) };
}

function setPixel(image, width, x, y, red, green, blue, alpha) {
  const offset = (y * width + x) * 4;
  image.data[offset] = red;
  image.data[offset + 1] = green;
  image.data[offset + 2] = blue;
  image.data[offset + 3] = alpha;
}

function getPixel(image, width, x, y) {
  const offset = (y * width + x) * 4;
  return Array.from(image.data.slice(offset, offset + 4));
}

function drawRectangle(image, width, left, top, right, bottom, alpha = 255) {
  for (let x = left; x <= right; x += 1) {
    setPixel(image, width, x, top, 20, 40, 60, alpha);
    setPixel(image, width, x, bottom, 20, 40, 60, alpha);
  }
  for (let y = top; y <= bottom; y += 1) {
    setPixel(image, width, left, y, 20, 40, 60, alpha);
    setPixel(image, width, right, y, 20, 40, 60, alpha);
  }
}

test("取り消し履歴は最新10件だけを保持する", () => {
  const history = [];
  for (let index = 0; index < 12; index += 1) {
    undoHistoryContext.add(history, { index }, 10);
  }

  assert.deepEqual(history.map((state) => state.index), [2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
  assert.equal(history.pop().index, 11);
  assert.equal(history.pop().index, 10);
});

test("バケツで線に囲まれた透明領域だけを選択色で塗る", () => {
  const width = 9;
  const image = createImage(width, 9);
  drawRectangle(image, width, 1, 1, 7, 7);
  drawRectangle(image, width, 2, 2, 6, 6);

  const changed = bucketContext.fill(image, width, 9, 4, 4, bucketContext.color("#e85d3f"));

  assert.equal(changed, true);
  assert.deepEqual(getPixel(image, width, 4, 4), [232, 93, 63, 255]);
  assert.deepEqual(getPixel(image, width, 2, 4), [232, 93, 63, 255]);
  assert.deepEqual(getPixel(image, width, 1, 4), [20, 40, 60, 255]);
  assert.deepEqual(getPixel(image, width, 0, 0), [0, 0, 0, 0]);
});

test("バケツは1pxの拡張範囲を越えた別領域へ広がらない", () => {
  const width = 5;
  const image = createImage(width, 5);
  for (let y = 0; y < 5; y += 1) {
    for (let x = 0; x < 5; x += 1) setPixel(image, width, x, y, 20, 40, 60, 255);
  }
  setPixel(image, width, 1, 1, 0, 0, 0, 0);
  setPixel(image, width, 3, 3, 0, 0, 0, 0);

  bucketContext.fill(image, width, 5, 3, 3, [255, 255, 255, 255]);

  assert.deepEqual(getPixel(image, width, 3, 3), [255, 255, 255, 255]);
  assert.deepEqual(getPixel(image, width, 1, 1), [0, 0, 0, 0]);
});

test("バケツを1px拡張して重ね描きで生じた不透明な境界色を上書きする", () => {
  const width = 6;
  const image = createImage(width, 1);
  setPixel(image, width, 1, 0, 255, 47, 0, 255);
  setPixel(image, width, 2, 0, 255, 190, 176, 255);
  for (let x = 3; x < 6; x += 1) setPixel(image, width, x, 0, 255, 193, 179, 255);

  bucketContext.fill(image, width, 1, 5, 0, [255, 47, 0, 255]);

  assert.deepEqual(getPixel(image, width, 3, 0), [255, 47, 0, 255]);
  assert.deepEqual(getPixel(image, width, 2, 0), [255, 47, 0, 255]);
  assert.deepEqual(getPixel(image, width, 1, 0), [255, 47, 0, 255]);
  assert.deepEqual(getPixel(image, width, 0, 0), [0, 0, 0, 0]);
});

test("バケツを1px拡張して半透明な境界色を上書きする", () => {
  const width = 5;
  const image = createImage(width, 1);
  setPixel(image, width, 1, 0, 20, 40, 60, 255);
  setPixel(image, width, 2, 0, 20, 40, 60, 128);

  bucketContext.fill(image, width, 1, 4, 0, [232, 93, 63, 255]);

  assert.deepEqual(getPixel(image, width, 3, 0), [232, 93, 63, 255]);
  assert.deepEqual(getPixel(image, width, 2, 0), [232, 93, 63, 255]);
  assert.deepEqual(getPixel(image, width, 1, 0), [20, 40, 60, 255]);
  assert.deepEqual(getPixel(image, width, 0, 0), [0, 0, 0, 0]);
});

test("閉じた図形の内側だけを白くする", () => {
  const width = 16;
  const image = createImage(width, 16);
  drawRectangle(image, width, 3, 3, 12, 12, 200);

  context.fill(image, width, 16);

  assert.deepEqual(getPixel(image, width, 7, 7), [255, 255, 255, 255]);
  assert.deepEqual(getPixel(image, width, 0, 0), [0, 0, 0, 0]);
  assert.deepEqual(getPixel(image, width, 3, 7), [20, 40, 60, 200]);
});

test("4pxの隙間を閉じて内側を白くする", () => {
  const width = 16;
  const image = createImage(width, 16);
  drawRectangle(image, width, 3, 3, 12, 12);
  for (let x = 6; x <= 9; x += 1) setPixel(image, width, x, 3, 0, 0, 0, 0);

  context.fill(image, width, 16);

  assert.deepEqual(getPixel(image, width, 7, 7), [255, 255, 255, 255]);
});

test("5pxの開口は閉じず内側を透過のままにする", () => {
  const width = 16;
  const image = createImage(width, 16);
  drawRectangle(image, width, 3, 3, 12, 12);
  for (let x = 6; x <= 10; x += 1) setPixel(image, width, x, 3, 0, 0, 0, 0);

  context.fill(image, width, 16);

  assert.deepEqual(getPixel(image, width, 7, 7), [0, 0, 0, 0]);
});

test("複数の閉じた図形を個別に白くする", () => {
  const width = 32;
  const image = createImage(width, 16);
  drawRectangle(image, width, 3, 3, 11, 12);
  drawRectangle(image, width, 20, 3, 28, 12);

  context.fill(image, width, 16);

  assert.deepEqual(getPixel(image, width, 7, 7), [255, 255, 255, 255]);
  assert.deepEqual(getPixel(image, width, 24, 7), [255, 255, 255, 255]);
  assert.deepEqual(getPixel(image, width, 15, 7), [0, 0, 0, 0]);
});

test("白塗り後にVRM用透過256pxとAI用白背景128pxを生成する", () => {
  const createdCanvases = [];
  const fillCalls = [];
  const submissionContext = vm.createContext({
    canvas: {},
    document: {
      createElement: () => {
        const operations = [];
        const fakeContext = {
          fillStyle: "",
          drawImage: (...args) => operations.push(["drawImage", ...args.slice(1)]),
          fillRect: (...args) => operations.push(["fillRect", fakeContext.fillStyle, ...args]),
          getImageData: () => ({}),
          putImageData: () => {},
        };
        const fakeCanvas = {
          width: 0,
          height: 0,
          operations,
          getContext: () => fakeContext,
        };
        createdCanvases.push(fakeCanvas);
        return fakeCanvas;
      },
    },
    fillEnclosedAreas: (_image, width, height) => fillCalls.push([width, height]),
  });
  vm.runInContext(
    `const CANVAS_SIZE = 512; const VRM_IMAGE_SIZE = 256; const AI_IMAGE_SIZE = 128; ${submissionSource}; this.create = createSubmissionCanvases;`,
    submissionContext,
  );

  const output = submissionContext.create();

  assert.equal(createdCanvases[0].width, 512);
  assert.equal(createdCanvases[0].height, 512);
  assert.deepEqual(fillCalls, [[512, 512]]);
  assert.equal(output.vrm.width, 256);
  assert.equal(output.vrm.height, 256);
  assert.deepEqual(createdCanvases[1].operations, [["drawImage", 0, 0, 256, 256]]);
  assert.equal(output.ai.width, 128);
  assert.equal(output.ai.height, 128);
  assert.deepEqual(createdCanvases[2].operations, [
    ["fillRect", "#fff", 0, 0, 128, 128],
    ["drawImage", 0, 0, 128, 128],
  ]);
});
