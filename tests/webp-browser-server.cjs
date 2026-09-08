const http = require("node:http");
const fs = require("node:fs");
const path = require("node:path");

const webRoot = path.resolve(__dirname, "../web");
http.createServer((request, response) => {
  const url = new URL(request.url, "http://localhost");
  const file = url.pathname.startsWith("/static/")
    ? path.resolve(webRoot, url.pathname.slice(8))
    : path.join(__dirname, "webp-browser.html");
  if (file !== path.join(__dirname, "webp-browser.html") && !file.startsWith(webRoot + path.sep)) {
    response.writeHead(403).end();
    return;
  }
  fs.readFile(file, (error, bytes) => {
    if (error) { response.writeHead(404).end(); return; }
    response.setHeader("Content-Type", file.endsWith(".wasm") ? "application/wasm"
      : file.endsWith(".js") ? "text/javascript" : "text/html; charset=utf-8");
    response.end(bytes);
  });
}).listen(8765, "127.0.0.1", () => {
  console.log("WebP検証: http://127.0.0.1:8765/ （WASM経路: /?wasm=1）");
});
