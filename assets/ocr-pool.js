// Pool of OCR Web Workers (see public/ocr-worker.js), driven from src/ocr.rs.
//
// Each worker loads the app's own wasm and its own copy of the OCR models.
// One worker finds the text lines; the lines are then recognised in small
// chunks handed out to whichever worker is free.
//
// For testing: localStorage "chipper.ocr.workers" = "0" turns the pool off
// (OCR runs on the main thread), or a number sets the worker count.
(() => {
  const MAX_WORKERS = 4;
  let pool = null; // Promise<Worker[]>
  let seq = 0;

  function workerCount() {
    let forced = null;
    try {
      forced = localStorage.getItem("chipper.ocr.workers");
    } catch {}
    if (forced !== null && forced !== "") return Math.max(0, parseInt(forced, 10) || 0);
    // Leave a core for the page.
    return Math.max(1, Math.min(MAX_WORKERS, (navigator.hardwareConcurrency || 2) - 1));
  }

  function spawn(url) {
    const w = new Worker(url, { type: "module" });
    w.pending = new Map();
    const failAll = (msg) => {
      for (const p of w.pending.values()) p.reject(new Error(msg));
      w.pending.clear();
    };
    w.onmessage = (e) => {
      const p = w.pending.get(e.data.id);
      if (!p) return;
      w.pending.delete(e.data.id);
      if (e.data.error !== undefined) p.reject(new Error(e.data.error));
      else p.resolve(e.data.result);
    };
    w.onerror = (e) => {
      e.preventDefault();
      failAll("OCR worker error: " + (e.message || "failed to load"));
    };
    w.onmessageerror = () => failAll("OCR worker message error");
    return w;
  }

  function call(w, op, args) {
    return new Promise((resolve, reject) => {
      const id = ++seq;
      w.pending.set(id, { resolve, reject });
      w.postMessage({ id, op, args });
    });
  }

  const abs = (url) => new URL(url, location.href).href;

  window.chipperOcr = {
    /** Start the workers (once); resolves to how many are running. */
    start(workerUrl, detectionUrl, recognitionUrl) {
      if (!pool) {
        pool = (async () => {
          const n = workerCount();
          if (n === 0) throw new Error("disabled by localStorage chipper.ocr.workers");
          if (typeof Worker === "undefined") throw new Error("Web Workers not supported");
          const script = document.querySelector('script[type="module"][src]');
          if (!script) throw new Error("app script not found");
          const args = { glue: script.src, det: abs(detectionUrl), rec: abs(recognitionUrl) };
          const workers = Array.from({ length: n }, () => spawn(abs(workerUrl)));
          try {
            await Promise.all(workers.map((w) => call(w, "init", args)));
          } catch (e) {
            workers.forEach((w) => w.terminate());
            throw e;
          }
          return workers;
        })();
        pool.catch(() => {
          pool = null;
        });
      }
      return pool.then((ws) => ws.length);
    },

    /** Stop all workers; the next start() spawns fresh ones. */
    reset() {
      if (pool) pool.then((ws) => ws.forEach((w) => w.terminate()), () => {});
      pool = null;
    },

    /** Give every worker the image to scan. */
    async setImage(rgba, w, h) {
      const ws = await pool;
      await Promise.all(ws.map((x) => call(x, "image", { rgba, w, h })));
    },

    /** Find the text lines (JSON), on the first worker. */
    async detect() {
      const ws = await pool;
      return call(ws[0], "detect");
    },

    /** Recognise each chunk (JSON line boxes); results come back in order. */
    async recognize(chunks, onProgress) {
      const ws = await pool;
      const out = new Array(chunks.length);
      let next = 0;
      let done = 0;
      await Promise.all(
        ws.map(async (w) => {
          while (next < chunks.length) {
            const i = next++;
            out[i] = await call(w, "recognize", { boxes: chunks[i] });
            onProgress(++done);
          }
        }),
      );
      return out;
    },
  };
})();
