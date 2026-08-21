const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const drawSource = fs.readFileSync(path.join(__dirname, "../web/js/draw.js"), "utf8");
const algorithmSource = drawSource.slice(
  drawSource.indexOf("function closeGaps"),
  drawSource.indexOf("function createSubmissionCanvas"),
);
const submissionSource = drawSource.slice(
  drawSource.indexOf("function createSubmissionCanvas"),
  drawSource.indexOf("function canvasBlob"),
);
const context = vm.createContext({});
vm.runInContext(
  `const ALPHA_THRESHOLD = 128; const GAP_CLOSE_RADIUS = 2; ${algorithmSource}; this.fill = fillEnclosedAreas;`,
  context,
);

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

test("512pxで白塗りしてから送信用の128pxへ縮小する", () => {
  const createdCanvases = [];
  const fillCalls = [];
  const submissionContext = vm.createContext({
    canvas: {},
    document: {
      createElement: () => {
        const drawCalls = [];
        const fakeCanvas = {
          width: 0,
          height: 0,
          drawCalls,
          getContext: () => ({
            drawImage: (...args) => drawCalls.push(args),
            getImageData: () => ({}),
            putImageData: () => {},
          }),
        };
        createdCanvases.push(fakeCanvas);
        return fakeCanvas;
      },
    },
    fillEnclosedAreas: (_image, width, height) => fillCalls.push([width, height]),
  });
  vm.runInContext(
    `const CANVAS_SIZE = 512; const SUBMISSION_SIZE = 128; ${submissionSource}; this.create = createSubmissionCanvas;`,
    submissionContext,
  );

  const output = submissionContext.create();

  assert.equal(createdCanvases[0].width, 512);
  assert.equal(createdCanvases[0].height, 512);
  assert.deepEqual(fillCalls, [[512, 512]]);
  assert.equal(output.width, 128);
  assert.equal(output.height, 128);
  assert.deepEqual(createdCanvases[1].drawCalls[0].slice(1), [0, 0, 128, 128]);
});
