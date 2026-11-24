export function initVenueSettings({ token, adminUrl, readError, setMessage }) {
  const byId = (name) => document.querySelector("#venue-" + name);
  const form = byId("form");
  const fields = byId("fields");
  const buildings = byId("buildings");
  const events = byId("events");
  const mapBuilding = byId("map-building");
  const map = byId("current-map");
  const pin = byId("edit-pin");
  const status = byId("status");
  const error = byId("error");

  function selectedBuilding() {
    return Array.from(buildings.children).find((row) => row.dataset.id === mapBuilding.value);
  }

  function showPin() {
    const row = selectedBuilding();
    pin.hidden = !row || !row.elements.x.validity.valid || !row.elements.y.validity.valid;
    if (!pin.hidden) {
      pin.style.left = row.elements.x.value + "%";
      pin.style.top = row.elements.y.value + "%";
    }
  }

  function fillOptions(select, selected) {
    const options = Array.from(buildings.children, (row) =>
      new Option(row.elements.namedItem("name").value.trim() || "名称未入力の棟", row.dataset.id));
    select.replaceChildren(new Option("棟を選択", ""), ...options);
    select.value = selected;
  }

  function updateBuildings(selected = mapBuilding.value) {
    fillOptions(mapBuilding, selected);
    if (!mapBuilding.value && buildings.firstElementChild) mapBuilding.selectedIndex = 1;
    for (const row of events.children) fillOptions(row.elements.building, row.elements.building.value);
    byId("add-event").disabled = !buildings.childElementCount;
    showPin();
  }

  function addRow(kind, id, values) {
    const isBuilding = kind === "building";
    const list = isBuilding ? buildings : events;
    const row = byId(kind + "-row").content.firstElementChild.cloneNode(true);
    row.dataset.id = id;
    for (const [key, value] of Object.entries(values)) {
      const input = row.elements.namedItem(key);
      if (key === "building") fillOptions(input, value);
      else input.value = value;
    }
    row.querySelector("button").addEventListener("click", () => {
      if (isBuilding && Array.from(events.children).some((event) => event.elements.building.value === id)) {
        setMessage(status, error, "この棟を使う企画があります。企画の棟を変更するか、企画を外してください。", true);
        return;
      }
      row.remove();
      if (isBuilding) updateBuildings();
    });
    if (isBuilding) {
      row.addEventListener("input", (event) => {
        if (event.target.name === "name") updateBuildings(id);
        else if (event.target.name === "x" || event.target.name === "y") {
          mapBuilding.value = id;
          showPin();
        }
      });
    }
    list.append(row);
    return row;
  }

  function nextId(kind, rows) {
    const used = new Set(Array.from(rows.children, (row) => row.dataset.id));
    let index = 1;
    while (used.has(kind + "-" + index)) index++;
    return kind + "-" + index;
  }

  byId("add-building").addEventListener("click", () => {
    const id = nextId("building", buildings);
    const row = addRow("building", id, { x: 50, y: 50 });
    updateBuildings(id);
    row.elements.namedItem("name").focus();
  });
  byId("add-event").addEventListener("click", () => {
    const row = addRow("event", nextId("event", events), { building: mapBuilding.value });
    row.elements.namedItem("name").focus();
  });
  mapBuilding.addEventListener("change", showPin);
  map.addEventListener("click", (event) => {
    const row = selectedBuilding();
    if (fields.disabled || !row) return;
    const rect = map.getBoundingClientRect();
    row.elements.x.value = ((event.clientX - rect.left) * 100 / rect.width).toFixed(2);
    row.elements.y.value = ((event.clientY - rect.top) * 100 / rect.height).toFixed(2);
    showPin();
  });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (!token || fields.disabled || !form.reportValidity()) return;
    const venue = {
      buildings: Object.fromEntries(Array.from(buildings.children, (row) => [row.dataset.id, {
        name: row.elements.namedItem("name").value.trim(),
        directions: row.elements.directions.value.trim(),
        pin: { x: row.elements.x.valueAsNumber, y: row.elements.y.valueAsNumber },
      }])),
      events: Object.fromEntries(Array.from(events.children, (row) => [row.dataset.id, {
        name: row.elements.namedItem("name").value.trim(),
        description: row.elements.description.value.trim(),
        building: row.elements.building.value,
        floor: row.elements.floor.value.trim(),
        room: row.elements.room.value.trim(),
      }])),
    };
    fields.disabled = true;
    setMessage(status, error);
    try {
      const response = await fetch(adminUrl("/api/admin/venue"), {
        method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(venue),
      });
      if (!response.ok) throw new Error(await readError(response, "会場案内を保存できませんでした。"));
      setMessage(status, error, "保存しました。次の質問から反映されます。");
    } catch (e) {
      setMessage(status, error, e.message, true);
    } finally {
      fields.disabled = false;
    }
  });

  return async function loadVenue() {
    fields.disabled = true;
    try {
      const response = await fetch(adminUrl("/api/admin/venue"), { cache: "no-store" });
      if (!response.ok) throw new Error(await readError(response, "会場案内を読み込めませんでした。"));
      const venue = await response.json();
      buildings.replaceChildren();
      events.replaceChildren();
      for (const [id, { name, directions, pin }] of Object.entries(venue?.buildings || {})) {
        addRow("building", id, { name, directions, ...pin });
      }
      for (const [id, value] of Object.entries(venue?.events || {})) addRow("event", id, value);
      updateBuildings();
      setMessage(status, error);
      fields.disabled = false;
    } catch (e) {
      setMessage(status, error, e.message, true);
    }
  };
}
