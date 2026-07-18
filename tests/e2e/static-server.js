// Minimal static server to serve the frontend (src/renderer) during the E2E tests.
// It adds nothing to the product: it just serves the files as-is, over HTTP, so Playwright
// can load them with the same origin the security policy expects (CSP script-src 'self').
// No external dependencies: it uses only Node's native modules.
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');

// The app's real frontend folder (the same one Tauri packages as frontendDist).
const ROOT = path.resolve(__dirname, '..', '..', 'src', 'renderer');
const PORT = Number(process.env.PORT || 4599);

// Content types for the files the frontend uses.
const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.json': 'application/json; charset=utf-8',
  '.ico': 'image/x-icon',
};

const server = http.createServer((req, res) => {
  // Normalizes the URL and prevents escaping ROOT (path traversal).
  let urlPath = decodeURIComponent((req.url || '/').split('?')[0]);
  if (urlPath === '/') urlPath = '/index.html';
  const filePath = path.normalize(path.join(ROOT, urlPath));
  if (!filePath.startsWith(ROOT)) {
    res.writeHead(403);
    res.end('Forbidden');
    return;
  }
  fs.readFile(filePath, (err, data) => {
    if (err) {
      res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
      res.end('Not found: ' + urlPath);
      return;
    }
    const type = TYPES[path.extname(filePath).toLowerCase()] || 'application/octet-stream';
    res.writeHead(200, { 'Content-Type': type });
    res.end(data);
  });
});

server.listen(PORT, () => {
  // Playwright expects this port (see playwright.config.js).
  console.log(`[static-server] serving ${ROOT} on http://127.0.0.1:${PORT}`);
});
