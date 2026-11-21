const assert = require("node:assert/strict");
const test = require("node:test");
const { setImmediate: nextTurn } = require("node:timers/promises");

function pageElement() {
  return Object.assign(new EventTarget(), {
    value: "", dataset: {}, style: { setProperty() {} }, classList: { contains: () => false },
    setAttribute() {}, setPointerCapture() {}, hasPointerCapture: () => false,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 512, height: 512 }),
  });
}

function canvas() {
  const image = (width, height) => ({ data: new Uint8ClampedArray(width * height * 4) });
  const context = {
    createImageData: image, getImageData: (_x, _y, width, height) => image(width, height),
    arc() {}, beginPath() {}, clearRect() {}, drawImage() {}, fillRect() {}, lineTo() {},
    moveTo() {}, putImageData() {}, restore() {}, save() {}, stroke() {}, strokeRect() {}, translate() {},
  };
  return Object.assign(pageElement(), {
    width: 512, height: 512, getContext: () => context,
    toBlob: (callback, type) => callback(new Blob(["image"], { type })),
  });
}

for (const question of [true, false]) {
  test(`${question ? "質問" : "描画"}は送信開始後1秒間の重複投稿を抑止し、次の投稿を受け付ける`, async (t) => {
    const elements = new Map([["#food-canvas", canvas()], ["#color-picker", canvas()]]);
    const document = Object.assign(new EventTarget(), {
      querySelector(id) {
        if (!elements.has(id)) elements.set(id, pageElement());
        return elements.get(id);
      },
      querySelectorAll: () => [], createElement: canvas,
    });
    const timers = [];
    globalThis.document = document;
    globalThis.window = {
      location: { pathname: "/event/test/draw" },
      setTimeout(callback, delay) { timers.push({ callback, delay }); },
    };
    t.after(() => { delete globalThis.document; delete globalThis.window; });

    const posts = [];
    t.mock.method(globalThis, "fetch", async (url, options) => {
      if (options.method === "POST") posts.push(url);
      return Response.json({ drawing: { stabilization: 3 } });
    });
    const form = document.querySelector("#submission-form");
    const text = document.querySelector("#text");
    form.reset = () => { text.value = ""; };
    if (question) {
      // NodeのFormDataにはHTMLFormElementからの生成がない。
      const NativeFormData = globalThis.FormData;
      globalThis.FormData = class extends NativeFormData { constructor() { super(); } };
      t.after(() => { globalThis.FormData = NativeFormData; });
      await import("../web/js/input.js");
    } else {
      await import("../web/js/draw.js");
    }
    const button = document.querySelector(question ? "#submit-button" : "#submit-food");
    const fill = () => {
      if (question) {
        text.value = "質問";
        text.dispatchEvent(new Event("input"));
      } else {
        const drawing = document.querySelector("#food-canvas");
        for (const type of ["pointerdown", "pointerup"]) {
          drawing.dispatchEvent(Object.assign(new Event(type), { pointerId: 1, clientX: 20, clientY: 20, button: 0 }));
        }
      }
    };
    const submit = () => question ? form.dispatchEvent(new Event("submit")) : button.dispatchEvent(new Event("click"));
    const endpoint = question ? "/event/test/api/submissions" : "/event/test/api/food-submissions";

    fill();
    submit();
    submit();
    await nextTurn();
    assert.deepEqual(posts, [endpoint]);
    fill();
    assert.equal(button.disabled, true);
    submit();
    await nextTurn();
    assert.deepEqual(posts, [endpoint]);
    assert.equal(timers.length, 1);
    assert.equal(timers[0].delay, 1000);
    timers[0].callback();
    assert.equal(button.disabled, false);
    submit();
    await nextTurn();
    assert.deepEqual(posts, [endpoint, endpoint]);
  });
}
