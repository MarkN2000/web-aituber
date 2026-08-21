import { createSourceButton } from "./sources.js";

export class ConversationHistory {
  constructor(root, list, onOpenSources) {
    this.root = root;
    this.list = list;
    this.onOpenSources = onOpenSources;
    this.hasRendered = false;
  }

  render(turns) {
    const wasNearBottom =
      !this.hasRendered || this.root.scrollHeight - this.root.scrollTop - this.root.clientHeight < 48;
    const fragment = document.createDocumentFragment();

    for (const turn of turns) {
      fragment.append(createTurn(turn, this.onOpenSources));
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

function createTurn(turn, onOpenSources) {
  const article = document.createElement("article");
  article.className = "history-turn";
  article.dataset.turnId = turn.turn_id;

  const question = document.createElement("p");
  question.className = "history-message history-message-user";
  question.setAttribute("aria-label", "ユーザーの質問");
  question.textContent = turn.question;

  const answer = document.createElement("p");
  answer.className = "history-message history-message-ai";
  answer.setAttribute("aria-label", "AIの回答");
  answer.textContent = turn.answer;
  const sourceButton = createSourceButton(turn.sources, onOpenSources);
  if (sourceButton) answer.append(sourceButton);

  article.append(question, answer);
  return article;
}
