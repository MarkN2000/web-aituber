const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const vm = require("node:vm");

function loadCooldown(fileName) {
  const source = fs.readFileSync(path.join(__dirname, `../web/js/${fileName}`), "utf8");
  const start = source.indexOf("function startSubmissionCooldown");
  const end = source.indexOf("\n}\n", start) + 3;
  const functionSource = source.slice(start, end);
  const timers = [];
  const context = vm.createContext({
    stateUpdates: 0,
    window: {
      setTimeout(callback, delay) {
        timers.push({ callback, delay });
      },
    },
  });
  vm.runInContext(`
    const SUBMISSION_COOLDOWN_MS = 1000;
    let isCoolingDown = false;
    function updateSubmitState() { stateUpdates += 1; }
    ${functionSource}
    this.start = startSubmissionCooldown;
    this.isActive = () => isCoolingDown;
  `, context);
  return {
    context,
    source,
    timers,
    stateUpdates: () => context.stateUpdates,
  };
}

for (const fileName of ["input.js", "draw.js"]) {
  test(`${fileName}は送信開始から1秒間の再送を抑止する`, () => {
    const cooldown = loadCooldown(fileName);

    cooldown.context.start();
    assert.equal(cooldown.context.isActive(), true);
    assert.equal(cooldown.timers.length, 1);
    assert.equal(cooldown.timers[0].delay, 1000);

    cooldown.timers[0].callback();
    assert.equal(cooldown.context.isActive(), false);
    assert.equal(cooldown.stateUpdates(), 1);
    assert.match(cooldown.source, /isSubmitting \|\| isCoolingDown/);
  });
}
