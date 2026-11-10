//! On-device OCR with [ocrs](https://github.com/robertknight/ocrs).
//!
//! The core (engine construction, recognition, row merging) is target
//! independent so it can be exercised natively against `tests/*.jpg`;
//! the browser glue (model download, canvas access, yielding) is wasm-only.

use ocrs::{ImageSource, OcrEngine, OcrEngineParams, OcrInput, TextItem, TextLine};
use rten_imageproc::RotatedRect;

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

/// Detect text and return line boxes plus the prepared input.
pub fn detect(engine: &OcrEngine, pixels: &[u8], w: u32, h: u32) -> Result<(OcrInput, Vec<Vec<RotatedRect>>), String> {
    let source = ImageSource::from_bytes(pixels, (w, h)).map_err(|e| e.to_string())?;
    let input = engine.prepare_input(source).map_err(|e| e.to_string())?;
    let words = engine.detect_words(&input).map_err(|e| e.to_string())?;
    let lines = engine.find_text_lines(&input, &words);
    Ok((input, lines))
}

/// Synchronous full pipeline (used by native tests).
#[allow(dead_code)]
pub fn recognize_pixels(engine: &OcrEngine, pixels: &[u8], w: u32, h: u32) -> Result<String, String> {
    let (input, lines) = detect(engine, pixels, w, h)?;
    let out = engine.recognize_text(&input, &lines).map_err(|e| e.to_string())?;
    Ok(merge_rows(&out.into_iter().flatten().collect::<Vec<_>>()))
}

fn char_center(c: &ocrs::TextChar) -> (f32, f32) {
    let r = c.rect;
    ((r.left() + r.right()) as f32 / 2.0, (r.top() + r.bottom()) as f32 / 2.0)
}

/// Dominant text slope (dy/dx) of the photo, from a least-squares fit of the
/// character centres of each reasonably long line, length-weighted median.
fn text_slope(lines: &[TextLine]) -> f32 {
    let mut fits: Vec<(f32, usize)> = lines
        .iter()
        .filter(|l| l.chars().len() >= 6)
        .filter_map(|l| {
            let pts: Vec<(f32, f32)> = l.chars().iter().filter(|c| c.char != ' ').map(char_center).collect();
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
pub fn merge_rows(lines: &[TextLine]) -> String {
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
            let text = l.to_string();
            let text = text.trim();
            if text.chars().count() < 2 && !text.chars().any(|c| c.is_ascii_digit()) {
                return None;
            }
            let pts: Vec<(f32, f32)> = l.chars().iter().filter(|c| c.char != ' ').map(char_center).collect();
            let n = pts.len().max(1) as f32;
            let y = pts.iter().map(|(x, y)| y - slope * x).sum::<f32>() / n;
            let height = l.chars().iter().map(|c| c.rect.height() as f32).sum::<f32>() / l.chars().len().max(1) as f32;
            Some(Span { y, height: height.max(1.0), left: l.bounding_rect().left() as f32, text: text.to_string() })
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
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    const DETECTION_MODEL: Asset = asset!("/assets/models/text-detection.onnx");
    const RECOGNITION_MODEL: Asset = asset!("/assets/models/text-recognition.onnx");

    thread_local! {
        static ENGINE: RefCell<Option<Rc<OcrEngine>>> = const { RefCell::new(None) };
    }

    fn js_err(e: JsValue) -> String {
        e.as_string().unwrap_or_else(|| format!("{e:?}"))
    }

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

    async fn engine(progress: &mut impl FnMut(f32, &str)) -> Result<Rc<OcrEngine>, String> {
        if let Some(e) = ENGINE.with_borrow(|e| e.clone()) {
            return Ok(e);
        }
        progress(0.03, "Downloading OCR models (≈12 MB, once)…");
        yield_now().await;
        let det = fetch_bytes(&DETECTION_MODEL.to_string()).await?;
        progress(0.08, "Downloading OCR models (≈12 MB, once)…");
        let rec = fetch_bytes(&RECOGNITION_MODEL.to_string()).await?;
        progress(0.12, "Loading OCR models…");
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

    /// Run OCR on the image currently on the hidden canvas and return the
    /// text, one receipt row per line.
    pub async fn recognize(mut progress: impl FnMut(f32, &str)) -> Result<String, String> {
        let engine = engine(&mut progress).await?;

        progress(0.15, "Finding text…");
        yield_now().await;
        let (w, h, rgba) = read_canvas()?;
        let (input, lines) = detect(&engine, &rgba, w, h)?;
        if lines.is_empty() {
            return Err("No text found in the image.".into());
        }

        let mut recognized: Vec<TextLine> = Vec::with_capacity(lines.len());
        const BATCH: usize = 8;
        for (i, chunk) in lines.chunks(BATCH).enumerate() {
            let done = (i * BATCH).min(lines.len());
            progress(0.25 + 0.73 * done as f32 / lines.len() as f32, &format!("Reading text… {done}/{}", lines.len()));
            yield_now().await;
            let out = engine.recognize_text(&input, chunk).map_err(|e| e.to_string())?;
            recognized.extend(out.into_iter().flatten());
        }
        progress(1.0, "Done");
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
    fn ocr_file(engine: &OcrEngine, name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(name);
        let img = image::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let s = crate::scan::target_scale(img.width(), img.height());
        let (w, h) = ((img.width() as f64 * s).round() as u32, (img.height() as f64 * s).round() as u32);
        let img = img.resize_exact(w, h, image::imageops::FilterType::CatmullRom);
        let rgb = img.into_rgb8();
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
