export class ConversationHistory {
  constructor(root, list) {
    this.root = root;
    this.list = list;
    this.hasRendered = false;
  }

  render(turns) {
    const wasNearBottom =
      !this.hasRendered || this.root.scrollHeight - this.root.scrollTop - this.root.clientHeight < 48;
    const fragment = document.createDocumentFragment();

    for (const turn of turns) {
      fragment.append(createTurn(turn));
    }

    this.list.replaceChildren(fragment);
    this.root.hidden = turns.length === 0;
    this.hasRendered = true;

    if (wasNearBottom) {
      requestAnimationFrame(() => {
        this.root.scrollTop = this.root.scrollHeight;
      });
    }
  }
}

function createTurn(turn) {
  const article = document.createElement("article");
  article.className = "history-turn";
  article.dataset.turnId = turn.turn_id;

  const question = document.createElement("p");
  question.className = "history-message history-message-user";
  question.setAttribute("aria-label", "ユーザーの質問");
  if (turn.has_image) {
    question.append(createImageIcon());
  }
  question.append(document.createTextNode(turn.question));

  const answer = document.createElement("p");
  answer.className = "history-message history-message-ai";
  answer.setAttribute("aria-label", "AIの回答");
  answer.textContent = turn.answer;

  article.append(question, answer);
  return article;
}

function createImageIcon() {
  const icon = document.createElement("span");
  icon.className = "history-image-icon";
  icon.setAttribute("role", "img");
  icon.setAttribute("aria-label", "画像付き");

  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");

  const frame = document.createElementNS("http://www.w3.org/2000/svg", "path");
  frame.setAttribute("d", "M4 5.5A1.5 1.5 0 0 1 5.5 4h13A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5z");
  const sun = document.createElementNS("http://www.w3.org/2000/svg", "circle");
  sun.setAttribute("cx", "9");
  sun.setAttribute("cy", "9");
  sun.setAttribute("r", "2");
  const mountain = document.createElementNS("http://www.w3.org/2000/svg", "path");
  mountain.setAttribute("d", "m5 17 4-4 3 3 2-2 5 5");

  svg.append(frame, sun, mountain);
  icon.append(svg);
  return icon;
}
