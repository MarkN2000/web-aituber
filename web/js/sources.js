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
  button.textContent = "🌐";
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
