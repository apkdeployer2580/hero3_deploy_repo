// Minimal offline service worker for the wie PWA.
//
// Strategy:
//  - Cross-origin requests (ads / analytics / CDN icons): pass straight through,
//    never touched or cached.
//  - Same-origin navigations (the HTML shell): network-first, so a fresh deploy
//    is picked up immediately; fall back to cache when offline.
//  - Same-origin static assets (content-hashed wasm/js/css, soundfont, icons):
//    cache-first for instant loads + full offline play after the first visit.
//
// Content-hashed filenames mean new builds fetch new URLs, so cache-first can
// never serve a stale bundle; only orphaned old entries linger (harmless).

const CACHE = "wie-cache-v1";

self.addEventListener("install", () => self.skipWaiting());

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)));
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;

  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return; // let cross-origin pass through untouched

  if (req.mode === "navigate") {
    // Network-first for the app shell so new deploys appear right away.
    event.respondWith(
      (async () => {
        try {
          const resp = await fetch(req);
          const cache = await caches.open(CACHE);
          cache.put(req, resp.clone());
          return resp;
        } catch {
          const cached = await caches.match(req);
          return cached || Response.error();
        }
      })(),
    );
    return;
  }

  // Cache-first for static assets.
  event.respondWith(
    (async () => {
      const cached = await caches.match(req);
      if (cached) return cached;
      const resp = await fetch(req);
      if (resp && resp.ok) {
        const cache = await caches.open(CACHE);
        cache.put(req, resp.clone());
      }
      return resp;
    })(),
  );
});
