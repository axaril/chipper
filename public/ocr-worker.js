// OCR worker (module). Loads the app's own wasm (whose `main` returns early
// when there is no `window`) and runs OCR through its `ocr_worker_*` exports.
// Started and driven by assets/ocr-pool.js. Lives in public/ so it is served
// unhashed at the site root.

let wasm = null;
let queue = Promise.resolve();

// One message at a time, in order: `init` is async and must finish before
// the `image` sent right after it is handled.
self.onmessage = (e) => {
  queue = queue.then(() => handle(e.data));
};

async function bytes(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`HTTP ${res.status} while downloading ${url}`);
  return new Uint8Array(await res.arrayBuffer());
}

async function handle({ id, op, args }) {
  try {
    let result;
    switch (op) {
      case "init": {
        wasm = await import(args.glue);
        await wasm.default();
        const [det, rec] = await Promise.all([bytes(args.det), bytes(args.rec)]);
        wasm.ocr_worker_init(det, rec);
        break;
      }
      case "image":
        wasm.ocr_worker_image(args.rgba, args.w, args.h);
        break;
      case "detect":
        result = wasm.ocr_worker_detect();
        break;
      case "recognize":
        result = wasm.ocr_worker_recognize(args.boxes);
        break;
      default:
        throw new Error("unknown op " + op);
    }
    self.postMessage({ id, result });
  } catch (err) {
    self.postMessage({ id, error: String((err && err.message) || err) });
  }
}
