//! On-device OCR with [ocrs](https://github.com/robertknight/ocrs).
//!
//! The core (engine construction, recognition, row merging) is target
//! independent so it can be exercised natively against `tests/*.jpg`.
//!
//! In the browser, the slow part (recognising each line) is spread over a
//! pool of Web Workers: `assets/ocr-pool.js` runs on the page and
//! `public/ocr-worker.js` in each worker. Every worker loads this same wasm
//! and drives it through the `ocr_worker_*` exports below, so there is no
//! separate build. If the pool can't start or fails mid-scan, the scan
//! runs on the main thread instead.

use ocrs::{ImageSource, OcrEngine, OcrEngineParams, OcrInput, TextLine};
use rten_imageproc::{PointF, RotatedRect, Vec2};
use serde::{Deserialize, Serialize};

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn build_engine(detection: Vec<u8>, recognition: Vec<u8>) -> Result<OcrEngine, String> {
    let detection_model = rten::Model::load(detection).map_err(|e| format!("detection model: {e}"))?;
    let recognition_model = rten::Model::load(recognition).map_err(|e| format!("recognition model: {e}"))?;
    OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })
    .map_err(|e| e.to_string())
}

/// Prepare an RGB(A) image for the engine.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn prepare(engine: &OcrEngine, pixels: &[u8], w: u32, h: u32) -> Result<OcrInput, String> {
    let source = ImageSource::from_bytes(pixels, (w, h)).map_err(|e| e.to_string())?;
    engine.prepare_input(source).map_err(|e| e.to_string())
}

/// Find the text lines; each line is the list of its word boxes.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn detect(engine: &OcrEngine, input: &OcrInput) -> Result<Vec<Vec<RotatedRect>>, String> {
    let words = engine.detect_words(input).map_err(|e| e.to_string())?;
    Ok(engine.find_text_lines(input, &words))
}

/// Recognise the given lines (unreadable ones are dropped).
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn recognize_lines(engine: &OcrEngine, input: &OcrInput, lines: &[Vec<RotatedRect>]) -> Result<Vec<Line>, String> {
    let out = engine.recognize_text(input, lines).map_err(|e| e.to_string())?;
    Ok(out.iter().flatten().map(Line::from).collect())
}

/// Synchronous full pipeline (used by native tests).
#[allow(dead_code)]
pub fn recognize_pixels(engine: &OcrEngine, pixels: &[u8], w: u32, h: u32) -> Result<String, String> {
    let input = prepare(engine, pixels, w, h)?;
    let lines = detect(engine, &input)?;
    Ok(merge_rows(&recognize_lines(engine, &input, &lines)?))
}

/// A recognised character and its box `[left, top, right, bottom]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glyph(pub char, pub [i32; 4]);

/// A recognised line of text. Plain data, so worker results can cross
/// threads as JSON.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Line(pub Vec<Glyph>);

impl From<&TextLine> for Line {
    fn from(l: &TextLine) -> Self {
        use ocrs::TextItem;
        Line(l.chars().iter().map(|c| Glyph(c.char, [c.rect.left(), c.rect.top(), c.rect.right(), c.rect.bottom()])).collect())
    }
}

impl Line {
    fn text(&self) -> String {
        self.0.iter().map(|g| g.0).collect()
    }

    fn centers(&self) -> Vec<(f32, f32)> {
        self.0
            .iter()
            .filter(|g| g.0 != ' ')
            .map(|Glyph(_, [l, t, r, b])| ((l + r) as f32 / 2.0, (t + b) as f32 / 2.0))
            .collect()
    }
}

/// A word box as `[cx, cy, up_x, up_y, width, height]`, for sending to a worker.
pub type WireBox = [f32; 6];

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn boxes_to_wire(lines: &[Vec<RotatedRect>]) -> Vec<Vec<WireBox>> {
    lines
        .iter()
        .map(|l| {
            l.iter()
                .map(|r| {
                    let (c, u) = (r.center(), r.up_axis());
                    [c.x, c.y, u.x, u.y, r.width(), r.height()]
                })
                .collect()
        })
        .collect()
}

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
pub fn boxes_from_wire(lines: &[Vec<WireBox>]) -> Vec<Vec<RotatedRect>> {
    lines
        .iter()
        .map(|l| {
            l.iter()
                .map(|&[cx, cy, ux, uy, w, h]| RotatedRect::new(PointF::from_yx(cy, cx), Vec2::from_xy(ux, uy), w, h))
                .collect()
        })
        .collect()
}

/// Dominant text slope (dy/dx) of the photo, from a least-squares fit of the
/// character centres of each reasonably long line, length-weighted median.
fn text_slope(lines: &[Line]) -> f32 {
    let mut fits: Vec<(f32, usize)> = lines
        .iter()
        .filter(|l| l.0.len() >= 6)
        .filter_map(|l| {
            let pts = l.centers();
            let n = pts.len() as f32;
            let mx = pts.iter().map(|p| p.0).sum::<f32>() / n;
            let my = pts.iter().map(|p| p.1).sum::<f32>() / n;
            let sxx: f32 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
            let sxy: f32 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
            (sxx > 1.0).then(|| (sxy / sxx, pts.len()))
        })
        .filter(|(s, _)| s.abs() < 0.35)
        .collect();
    if fits.is_empty() {
        return 0.0;
    }
    fits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let half = fits.iter().map(|f| f.1).sum::<usize>() / 2;
    let mut acc = 0;
    for (s, w) in &fits {
        acc += w;
        if acc >= half {
            return *s;
        }
    }
    fits.last().unwrap().0
}

/// ocrs splits a receipt row like `TOFU ........ 6.99` into separate lines
/// because of the wide gap. Re-join lines that sit at the same height, after
/// compensating for the photo being tilted.
pub fn merge_rows(lines: &[Line]) -> String {
    struct Span {
        /// Vertical centre after removing the global slope.
        y: f32,
        height: f32,
        left: f32,
        text: String,
    }
    let slope = text_slope(lines);
    let mut spans: Vec<Span> = lines
        .iter()
        .filter_map(|l| {
            let text = l.text();
            let text = text.trim();
            if text.chars().count() < 2 && !text.chars().any(|c| c.is_ascii_digit()) {
                return None;
            }
            let pts = l.centers();
            let n = pts.len().max(1) as f32;
            let y = pts.iter().map(|(x, y)| y - slope * x).sum::<f32>() / n;
            let height = l.0.iter().map(|Glyph(_, [_, t, _, b])| (b - t) as f32).sum::<f32>() / l.0.len().max(1) as f32;
            let left = l.0.iter().map(|g| g.1[0]).min().unwrap_or(0) as f32;
            Some(Span { y, height: height.max(1.0), left, text: text.to_string() })
        })
        .collect();
    spans.sort_by(|a, b| a.y.total_cmp(&b.y));

    let mut rows: Vec<(f32, f32, Vec<Span>)> = Vec::new(); // (mean y, mean height, spans)
    for s in spans {
        match rows.last_mut() {
            Some((y, h, row)) if (s.y - *y).abs() < 0.5 * h.max(s.height) => {
                let n = row.len() as f32;
                *y = (*y * n + s.y) / (n + 1.0);
                *h = (*h * n + s.height) / (n + 1.0);
                row.push(s);
            }
            _ => rows.push((s.y, s.height, vec![s])),
        }
    }
    rows.into_iter()
        .map(|(_, _, mut row)| {
            row.sort_by(|a, b| a.left.total_cmp(&b.left));
            row.into_iter().map(|s| s.text).collect::<Vec<_>>().join("   ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(target_arch = "wasm32")]
pub use web::recognize;

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use crate::scan::CANVAS_ID;
    use dioxus::prelude::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;

    const DETECTION_MODEL: Asset = asset!("/assets/models/text-detection.onnx");
    const RECOGNITION_MODEL: Asset = asset!("/assets/models/text-recognition.onnx");
    /// Lines per work item handed to a worker. Small, so faster workers pick
    /// up more items and all finish at about the same time.
    const CHUNK: usize = 4;

    thread_local! {
        /// Main-thread engine, only built when the worker pool is unavailable.
        static ENGINE: RefCell<Option<Rc<OcrEngine>>> = const { RefCell::new(None) };
        /// Inside a worker: its engine and the image being scanned.
        static WORKER: RefCell<Option<(OcrEngine, Option<OcrInput>)>> = const { RefCell::new(None) };
    }

    type Progress = Rc<RefCell<dyn FnMut(f32, &str)>>;

    fn say(p: &Progress, fraction: f32, label: &str) {
        (p.borrow_mut())(fraction, label);
    }

    fn js_err(e: JsValue) -> String {
        e.as_string()
            .or_else(|| e.dyn_ref::<js_sys::Error>().map(|e| String::from(e.message())))
            .unwrap_or_else(|| format!("{e:?}"))
    }

    // ---- worker side: called from public/ocr-worker.js ----

    fn worker_err(e: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&e.to_string())
    }

    #[wasm_bindgen]
    pub fn ocr_worker_init(detection: Vec<u8>, recognition: Vec<u8>) -> Result<(), JsValue> {
        let engine = build_engine(detection, recognition).map_err(worker_err)?;
        WORKER.with_borrow_mut(|w| *w = Some((engine, None)));
        Ok(())
    }

    #[wasm_bindgen]
    pub fn ocr_worker_image(rgba: &[u8], w: u32, h: u32) -> Result<(), JsValue> {
        WORKER.with_borrow_mut(|slot| {
            let (engine, input) = slot.as_mut().ok_or_else(|| worker_err("worker not initialised"))?;
            *input = Some(prepare(engine, rgba, w, h).map_err(worker_err)?);
            Ok(())
        })
    }

    fn with_input(f: impl FnOnce(&OcrEngine, &OcrInput) -> Result<String, String>) -> Result<String, JsValue> {
        WORKER.with_borrow(|slot| match slot {
            Some((engine, Some(input))) => f(engine, input).map_err(worker_err),
            _ => Err(worker_err("no image set")),
        })
    }

    /// Detect lines; returns `Vec<Vec<WireBox>>` as JSON.
    #[wasm_bindgen]
    pub fn ocr_worker_detect() -> Result<String, JsValue> {
        with_input(|e, i| serde_json::to_string(&boxes_to_wire(&detect(e, i)?)).map_err(|e| e.to_string()))
    }

    /// Recognise lines given as `Vec<Vec<WireBox>>` JSON; returns `Vec<Line>` JSON.
    #[wasm_bindgen]
    pub fn ocr_worker_recognize(boxes: &str) -> Result<String, JsValue> {
        let wire: Vec<Vec<WireBox>> = serde_json::from_str(boxes).map_err(worker_err)?;
        with_input(|e, i| serde_json::to_string(&recognize_lines(e, i, &boxes_from_wire(&wire))?).map_err(|e| e.to_string()))
    }

    // ---- page side: the pool in assets/ocr-pool.js ----

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = chipperOcr, js_name = start, catch)]
        async fn pool_start(worker_url: &str, detection_url: &str, recognition_url: &str) -> Result<JsValue, JsValue>;
        #[wasm_bindgen(js_namespace = chipperOcr, js_name = reset, catch)]
        fn pool_reset() -> Result<(), JsValue>;
        #[wasm_bindgen(js_namespace = chipperOcr, js_name = setImage, catch)]
        async fn pool_set_image(rgba: js_sys::Uint8Array, w: u32, h: u32) -> Result<JsValue, JsValue>;
        #[wasm_bindgen(js_namespace = chipperOcr, js_name = detect, catch)]
        async fn pool_detect() -> Result<JsValue, JsValue>;
        #[wasm_bindgen(js_namespace = chipperOcr, js_name = recognize, catch)]
        async fn pool_recognize(chunks: js_sys::Array, on_progress: js_sys::Function) -> Result<JsValue, JsValue>;
    }

    /// Site root, including any `--base-path`: whatever precedes `assets/`.
    fn site_base() -> String {
        let model = DETECTION_MODEL.to_string();
        model.find("assets/").map_or("/".into(), |i| model[..i].to_string())
    }

    /// Run OCR on the image currently on the hidden canvas and return the
    /// text, one receipt row per line.
    pub async fn recognize(progress: impl FnMut(f32, &str) + 'static) -> Result<String, String> {
        let progress: Progress = Rc::new(RefCell::new(progress));
        let (w, h, rgba) = read_canvas()?;
        match pooled(&progress, &rgba, w, h).await {
            Ok(result) => result,
            Err(e) => {
                web_sys::console::warn_1(&format!("OCR worker pool unavailable, using the main thread: {e}").into());
                let _ = pool_reset();
                single_thread(&progress, &rgba, w, h).await
            }
        }
    }

    /// `Ok(result)`: the pool did the work (the result may still be an error
    /// such as "no text"). `Err`: the pool is unavailable or broke, so the
    /// caller falls back to the main thread.
    async fn pooled(p: &Progress, rgba: &[u8], w: u32, h: u32) -> Result<Result<String, String>, String> {
        say(p, 0.03, "Loading OCR models…");
        let threads = pool_start(&format!("{}ocr-worker.js", site_base()), &DETECTION_MODEL.to_string(), &RECOGNITION_MODEL.to_string())
            .await
            .map_err(js_err)?
            .as_f64()
            .unwrap_or(1.0) as usize;

        say(p, 0.12, "Finding text…");
        pool_set_image(js_sys::Uint8Array::from(rgba), w, h).await.map_err(js_err)?;
        let detected = pool_detect().await.map_err(js_err)?.as_string().unwrap_or_default();
        let boxes: Vec<Vec<WireBox>> = serde_json::from_str(&detected).map_err(|e| e.to_string())?;
        if boxes.is_empty() {
            return Ok(Err("No text found in the image.".into()));
        }

        let total = boxes.len();
        let label = move |lines: usize| format!("Reading text… {lines}/{total} ({threads} threads)");
        say(p, 0.25, &label(0));
        let chunks = js_sys::Array::new();
        for c in boxes.chunks(CHUNK) {
            chunks.push(&JsValue::from_str(&serde_json::to_string(c).map_err(|e| e.to_string())?));
        }
        let pp = p.clone();
        let on_progress = Closure::<dyn FnMut(u32)>::new(move |chunks_done: u32| {
            let lines = (chunks_done as usize * CHUNK).min(total);
            say(&pp, 0.25 + 0.73 * lines as f32 / total as f32, &label(lines));
        });
        let parts = pool_recognize(chunks, on_progress.as_ref().unchecked_ref::<js_sys::Function>().clone())
            .await
            .map_err(js_err)?;
        drop(on_progress);

        let mut lines = Vec::with_capacity(total);
        for part in js_sys::Array::from(&parts).iter() {
            let part: Vec<Line> = serde_json::from_str(&part.as_string().unwrap_or_default()).map_err(|e| e.to_string())?;
            lines.extend(part);
        }
        say(p, 1.0, "Done");
        Ok(Ok(merge_rows(&lines)))
    }

    // ---- main-thread fallback ----

    /// Let the browser paint (progress bar updates) between heavy steps.
    ///
    /// Uses a MessageChannel task rather than `setTimeout`, which browsers
    /// throttle to ~1/s (or worse) in background tabs.
    async fn yield_now() {
        thread_local! {
            static MAKE: js_sys::Function = js_sys::Function::new_no_args(
                "return new Promise(r => { const c = new MessageChannel(); \
                 c.port1.onmessage = () => r(); c.port2.postMessage(0); });",
            );
        }
        if let Ok(p) = MAKE.with(|f| f.call0(&JsValue::NULL)) {
            let _ = JsFuture::from(js_sys::Promise::from(p)).await;
        }
    }

    async fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
        let window = web_sys::window().ok_or("no window")?;
        let resp: web_sys::Response = JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(js_err)?
            .dyn_into()
            .map_err(js_err)?;
        if !resp.ok() {
            return Err(format!("HTTP {} while downloading {url}", resp.status()));
        }
        let buf = JsFuture::from(resp.array_buffer().map_err(js_err)?).await.map_err(js_err)?;
        Ok(js_sys::Uint8Array::new(&buf).to_vec())
    }

    async fn engine(p: &Progress) -> Result<Rc<OcrEngine>, String> {
        if let Some(e) = ENGINE.with_borrow(|e| e.clone()) {
            return Ok(e);
        }
        say(p, 0.03, "Downloading OCR models (≈12 MB, first scan only)…");
        yield_now().await;
        let det = fetch_bytes(&DETECTION_MODEL.to_string()).await?;
        say(p, 0.08, "Downloading OCR models (≈12 MB, first scan only)…");
        let rec = fetch_bytes(&RECOGNITION_MODEL.to_string()).await?;
        say(p, 0.12, "Loading OCR models…");
        yield_now().await;
        let engine = Rc::new(build_engine(det, rec)?);
        ENGINE.with_borrow_mut(|slot| *slot = Some(engine.clone()));
        Ok(engine)
    }

    fn read_canvas() -> Result<(u32, u32, Vec<u8>), String> {
        let doc = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
        let canvas: web_sys::HtmlCanvasElement = doc
            .get_element_by_id(CANVAS_ID)
            .ok_or("no image loaded")?
            .dyn_into()
            .map_err(|_| "not a canvas")?;
        let ctx: web_sys::CanvasRenderingContext2d = canvas
            .get_context("2d")
            .map_err(js_err)?
            .ok_or("no 2d context")?
            .dyn_into()
            .map_err(|_| "not a 2d context")?;
        let (w, h) = (canvas.width(), canvas.height());
        let data = ctx.get_image_data(0.0, 0.0, w as f64, h as f64).map_err(js_err)?;
        Ok((w, h, data.data().0))
    }

    async fn single_thread(p: &Progress, rgba: &[u8], w: u32, h: u32) -> Result<String, String> {
        let engine = engine(p).await?;
        say(p, 0.15, "Finding text…");
        yield_now().await;
        let input = prepare(&engine, rgba, w, h)?;
        let lines = detect(&engine, &input)?;
        if lines.is_empty() {
            return Err("No text found in the image.".into());
        }
        let mut recognized = Vec::with_capacity(lines.len());
        const BATCH: usize = 8;
        for (i, chunk) in lines.chunks(BATCH).enumerate() {
            let done = (i * BATCH).min(lines.len());
            say(p, 0.25 + 0.73 * done as f32 / lines.len() as f32, &format!("Reading text… {done}/{}", lines.len()));
            yield_now().await;
            recognized.extend(recognize_lines(&engine, &input, chunk)?);
        }
        say(p, 1.0, "Done");
        Ok(merge_rows(&recognized))
    }
}

#[cfg(test)]
mod tests {
    //! Run with `cargo test --release ocr_samples -- --ignored --nocapture`
    //! to print what ocrs + the parser make of each photo in `tests/`.
    use super::*;
    use crate::receipt::{ParsedReceipt, parse_text};
    use std::path::Path;

    fn engine() -> OcrEngine {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/models");
        build_engine(
            std::fs::read(dir.join("text-detection.onnx")).unwrap(),
            std::fs::read(dir.join("text-recognition.onnx")).unwrap(),
        )
        .unwrap()
    }

    /// Mirror the browser's `scan::target_scale` resizing.
    fn load(name: &str) -> image::RgbImage {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(name);
        let img = image::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let s = crate::scan::target_scale(img.width(), img.height());
        let (w, h) = ((img.width() as f64 * s).round() as u32, (img.height() as f64 * s).round() as u32);
        img.resize_exact(w, h, image::imageops::FilterType::CatmullRom).into_rgb8()
    }

    fn ocr_file(engine: &OcrEngine, name: &str) -> String {
        let rgb = load(name);
        recognize_pixels(engine, rgb.as_raw(), rgb.width(), rgb.height()).unwrap()
    }

    fn report(name: &str, text: &str) -> ParsedReceipt {
        let r = parse_text(text);
        println!("\n===== {name} =====\n{text}\n----- parsed -----");
        for it in &r.items {
            println!("  {:<28} {}", it.name, it.price_expr());
        }
        println!("  subtotal={:?} tax={:?} tip={:?} total={:?}", r.subtotal, r.tax, r.tip, r.total);
        r
    }

    /// Longest common subsequence of expected vs parsed item prices.
    fn matched(r: &ParsedReceipt, want: &[f64]) -> usize {
        let got: Vec<f64> = r.items.iter().map(|i| i.price).collect();
        let mut dp = vec![vec![0usize; got.len() + 1]; want.len() + 1];
        for i in 1..=want.len() {
            for j in 1..=got.len() {
                dp[i][j] = if (want[i - 1] - got[j - 1]).abs() < 0.005 {
                    dp[i - 1][j - 1] + 1
                } else {
                    dp[i - 1][j].max(dp[i][j - 1])
                };
            }
        }
        dp[want.len()][got.len()]
    }

    /// Splitting lines into chunks and passing boxes/results through JSON,
    /// as the worker pool does, must give exactly the single-pass text.
    #[test]
    #[ignore = "slow: runs the OCR models"]
    fn chunked_matches_single_pass() {
        let engine = engine();
        let rgb = load("wholefoods-tribeca.jpg");
        let input = prepare(&engine, rgb.as_raw(), rgb.width(), rgb.height()).unwrap();
        let lines = detect(&engine, &input).unwrap();
        let whole = merge_rows(&recognize_lines(&engine, &input, &lines).unwrap());

        let wire: Vec<Vec<WireBox>> = serde_json::from_str(&serde_json::to_string(&boxes_to_wire(&lines)).unwrap()).unwrap();
        let mut parts = Vec::new();
        for chunk in wire.chunks(4) {
            let got = recognize_lines(&engine, &input, &boxes_from_wire(chunk)).unwrap();
            let json = serde_json::to_string(&got).unwrap();
            parts.extend(serde_json::from_str::<Vec<Line>>(&json).unwrap());
        }
        assert_eq!(merge_rows(&parts), whole);
    }

    #[test]
    #[ignore = "slow: runs the OCR models"]
    fn ocr_samples() {
        let engine = engine();
        let mut failures = vec![];
        // (file, item prices before discounts, total). Up to a quarter of
        // prices (rounded up) may be misread by OCR; totals must come out right.
        let cases: &[(&str, &[f64], f64)] = &[
            ("costco-greenville.jpg", &[22.10, 5.99, 10.39], 38.48),
            ("costco-rochester.webp", &[6.99, 1.94, 18.36, 6.99, 5.49, 7.99, 5.99, 31.42], 79.34),
            ("costco-santee.jpg", &[18.99, 14.99, 1.20, 32.99, 8.99, 8.99, 14.99, 5.89, 6.39, 10.99, 4.97, 5.99, 12.99, 6.69, 4.99, 3.49], 163.57),
            ("costco-leesburg.jpg", &[39.99, 7.99, 7.89, 8.29, 9.99, 11.79, 8.99, 4.99, 8.99, 7.99, 1.99], 123.00),
            ("costco-thornton.jpg", &[23.99, 6.49, 2.97, 12.87, 6.29, 6.49, 18.47, 3.59, 4.45], 89.13),
            ("kroger-north-beach.jpg", &[3.45, 1.23, 1.59, 2.49, 0.99, 1.69], 11.44),
            ("wholefoods-cambridge.jpg", &[3.50], 3.89),
            ("wholefoods-tribeca.jpg", &[6.99, 1.99, 3.99, 4.99, 7.99, 2.99, 0.99, 1.20, 2.98, 3.49, 0.91, 2.99, 3.16, 0.77, 2.50, 1.69], 50.33),
        ];
        for (file, want, total) in cases {
            let r = report(file, &ocr_file(&engine, file));
            let m = matched(&r, want);
            println!("  => {m}/{} prices, total {:?} (want {total})", want.len(), r.total);
            let total_ok = r.total.is_some_and(|t| (t - total).abs() < 0.1);
            if want.len() - m > want.len().div_ceil(4) || r.items.len() > want.len() + 1 || !total_ok {
                failures.push(format!("{file}: {m}/{} prices", want.len()));
            }
        }

        assert!(failures.is_empty(), "mismatches: {failures:?}");
    }
}
