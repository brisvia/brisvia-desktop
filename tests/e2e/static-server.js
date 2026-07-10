// Servidor estático mínimo para servir el frontend (src/renderer) durante los tests E2E.
// No agrega nada al producto: solo sirve los archivos tal cual, sobre HTTP, para que Playwright
// pueda cargarlos con el mismo origen que espera la política de seguridad (CSP script-src 'self').
// Sin dependencias externas: usa solo módulos nativos de Node.
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');

// Carpeta del frontend real de la app (la misma que Tauri empaqueta como frontendDist).
const ROOT = path.resolve(__dirname, '..', '..', 'src', 'renderer');
const PORT = Number(process.env.PORT || 4599);

// Tipos de contenido para los archivos que usa el frontend.
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
  // Normaliza la URL y evita salir de ROOT (path traversal).
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
      res.end('No encontrado: ' + urlPath);
      return;
    }
    const type = TYPES[path.extname(filePath).toLowerCase()] || 'application/octet-stream';
    res.writeHead(200, { 'Content-Type': type });
    res.end(data);
  });
});

server.listen(PORT, () => {
  // Playwright espera este puerto (ver playwright.config.js).
  console.log(`[static-server] sirviendo ${ROOT} en http://127.0.0.1:${PORT}`);
});
