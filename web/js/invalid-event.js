const STYLESHEET_URL = "/static/invalid-event.css?v=1";

export const INVALID_EVENT_MESSAGE = "このURLは使用できません。";

export function showInvalidEventScreen() {
  if (!document.querySelector(`link[href="${STYLESHEET_URL}"]`)) {
    const stylesheet = document.createElement("link");
    stylesheet.rel = "stylesheet";
    stylesheet.href = STYLESHEET_URL;
    document.head.append(stylesheet);
  }

  const message = document.createElement("main");
  message.className = "invalid-event-message";
  message.setAttribute("role", "alert");
  message.textContent = INVALID_EVENT_MESSAGE;
  document.body.className = "invalid-event-page";
  document.body.replaceChildren(message);
}
