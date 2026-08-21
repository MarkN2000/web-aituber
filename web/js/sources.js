export class SourceDialog {
  constructor(dialog, list, closeButton) {
    this.dialog = dialog;
    this.list = list;
    closeButton.addEventListener("click", () => dialog.close());
    dialog.addEventListener("click", (event) => {
      if (event.target === dialog) dialog.close();
    });
  }

  open(sources) {
    const validSources = normalizeSources(sources);
    if (!validSources.length) return;

    const fragment = document.createDocumentFragment();
    for (const source of validSources) {
      const item = document.createElement("li");
      const link = document.createElement("a");
      link.href = source.url;
      link.textContent = source.url;
      link.title = source.title || source.url;
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      item.append(link);
      fragment.append(item);
    }
    this.list.replaceChildren(fragment);
    this.dialog.showModal();
  }
}

export function createSourceButton(sources, onOpen) {
  const validSources = normalizeSources(sources);
  if (!validSources.length) return undefined;

  const button = document.createElement("button");
  button.className = "source-button";
  button.type = "button";
  button.setAttribute("aria-label", `検索に使用した出典を表示（${validSources.length}件）`);
  button.setAttribute("aria-haspopup", "dialog");
  button.title = "出典を表示";
  button.append(createGlobeIcon());
  button.addEventListener("click", () => onOpen(validSources));
  return button;
}

function normalizeSources(sources) {
  const unique = new Map();
  for (const source of Array.isArray(sources) ? sources : []) {
    if (!source || typeof source.url !== "string") continue;
    let url;
    try {
      url = new URL(source.url);
    } catch {
      continue;
    }
    if (url.protocol !== "https:" && url.protocol !== "http:") continue;
    if (!unique.has(url.href)) {
      unique.set(url.href, {
        url: url.href,
        title: typeof source.title === "string" ? source.title : "",
      });
    }
  }
  return [...unique.values()];
}

function createGlobeIcon() {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");

  const circle = document.createElementNS("http://www.w3.org/2000/svg", "circle");
  circle.setAttribute("cx", "12");
  circle.setAttribute("cy", "12");
  circle.setAttribute("r", "9");
  const meridian = document.createElementNS("http://www.w3.org/2000/svg", "path");
  meridian.setAttribute("d", "M12 3c2.4 2.5 3.6 5.5 3.6 9S14.4 18.5 12 21c-2.4-2.5-3.6-5.5-3.6-9S9.6 5.5 12 3Z");
  const latitude = document.createElementNS("http://www.w3.org/2000/svg", "path");
  latitude.setAttribute("d", "M3.5 9h17M3.5 15h17");
  svg.append(circle, meridian, latitude);
  return svg;
}
