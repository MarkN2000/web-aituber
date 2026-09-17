const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "../web/js/draw.js"), "utf8");

test("タブの往復で状態を保持し、キーボードでも切り替えられる", () => {
  const panels = {
    "panel-draw": { hidden: false, drawing: "描きかけ", undoHistory: ["前の絵"] },
    "panel-question": { hidden: true, text: "入力途中の質問" },
  };
  let focused;
  const tabs = Object.keys(panels).map((id) => ({
    getAttribute: () => id,
    setAttribute(name, value) { this[name] = value; },
    focus() { focused = this; },
  }));
  let stoppedDrawing;
  let stoppedColor;
  let undoCount = 0;
  const context = vm.createContext({
    tabs, drawPanel: panels["panel-draw"], drawCursor: { hidden: false },
    activePointer: 7, colorPointer: 8, eventEnded: false, undoButton: { disabled: false },
    document: { getElementById: (id) => panels[id] },
    stopDrawing(event) { stoppedDrawing = event; context.activePointer = undefined; },
    stopColorPicking(event) { stoppedColor = event; context.colorPointer = undefined; },
    undoCanvas() { undoCount += 1; },
  });
  vm.runInContext(source.slice(source.indexOf("function activateTab"), source.indexOf("async function loadDrawingConfig")), context);
  vm.runInContext(source.slice(source.indexOf("function isUndoShortcut"), source.indexOf("function addUndoState")), context);

  context.activateTab(tabs[1]);
  assert.equal(stoppedDrawing.pointerId, 7);
  assert.equal(stoppedDrawing.type, "pointercancel");
  assert.equal(stoppedColor.pointerId, 8);
  assert.equal(context.drawCursor.hidden, true);
  assert.equal(panels["panel-draw"].hidden, true);
  assert.equal(panels["panel-question"].hidden, false);
  assert.equal(tabs[1]["aria-selected"], "true");
  assert.equal(tabs[0].tabIndex, -1);
  assert.equal(tabs[1].tabIndex, 0);

  const undoEvent = { key: "z", ctrlKey: true, preventDefault() { this.defaultPrevented = true; } };
  context.handleUndoShortcut(undoEvent);
  assert.equal(undoCount, 0);
  assert.equal(undoEvent.defaultPrevented, undefined);

  for (const [key, from, to] of [["ArrowRight", 1, 0], ["ArrowLeft", 0, 1], ["Home", 1, 0], ["End", 0, 1]]) {
    let prevented = false;
    context.tabKeydown({ key, currentTarget: tabs[from], preventDefault() { prevented = true; } });
    assert.equal(prevented, true);
    assert.equal(focused, tabs[to]);
    assert.equal(tabs[to]["aria-selected"], "true");
  }
  context.activateTab(tabs[0]);
  assert.equal(panels["panel-draw"].hidden, false);
  assert.equal(panels["panel-question"].hidden, true);
  assert.equal(panels["panel-draw"].drawing, "描きかけ");
  assert.deepEqual(panels["panel-draw"].undoHistory, ["前の絵"]);
  assert.equal(panels["panel-question"].text, "入力途中の質問");
  context.handleUndoShortcut(undoEvent);
  assert.equal(undoCount, 1);
  assert.equal(undoEvent.defaultPrevented, true);
});
