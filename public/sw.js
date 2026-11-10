// Chipper service worker.
//
// dx names every bundled file with a content hash (`name-dxh<hash>.ext`), so
// those are immutable: serve them cache-first and keep them. That covers the
// ≈12 MB OCR models as well as the wasm, JS and CSS. When a new build ships a
// file under a new hash, older copies with the same name are dropped.
//
// Page loads (index.html) go network-first, falling back to the cached copy,
// so the app also opens offline.

const CACHE = "chipper-v1";
const HASHED = /-dxh[0-9a-f]+(\.[a-z0-9]+)$/i;

self.addEventListener("install", () => self.skipWaiting());

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      for (const name of await caches.keys()) {
        if (name.startsWith("chipper-") && name !== CACHE) await caches.delete(name);
      }
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET" || req.headers.has("range")) return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (HASHED.test(url.pathname)) {
    event.respondWith(cacheFirst(req, url));
  } else if (req.mode === "navigate") {
    event.respondWith(networkFirst(req));
  }
});

/** Path with the content hash removed: `/assets/main-dxh12ab.css` → `/assets/main.css`. */
const stem = (path) => path.replace(HASHED, "$1");

async function cacheFirst(req, url) {
  const cache = await caches.open(CACHE);
  const hit = await cache.match(req, { ignoreSearch: true });
  if (hit) return hit;
  const res = await fetch(req);
  if (res.status === 200) {
    await cache.put(req, res.clone());
    // Drop superseded builds of the same file.
    const mine = stem(url.pathname);
    for (const key of await cache.keys()) {
      const path = new URL(key.url).pathname;
      if (path !== url.pathname && HASHED.test(path) && stem(path) === mine) await cache.delete(key);
    }
  }
  return res;
}

async function networkFirst(req) {
  const cache = await caches.open(CACHE);
  try {
    const res = await fetch(req);
    if (res.status === 200) await cache.put(req, res.clone());
    return res;
  } catch (err) {
    const hit =
      (await cache.match(req, { ignoreSearch: true })) ||
      (await cache.match(new URL("./", self.registration.scope).href));
    if (hit) return hit;
    throw err;
  }
}
