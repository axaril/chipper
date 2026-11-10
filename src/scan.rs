//! Browser glue: reading the chosen image onto a canvas, perspective crop,
//! clipboard.

use dioxus::prelude::*;
use serde::Deserialize;

/// Hidden canvas holding the flattened receipt that OCR reads.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const CANVAS_ID: &str = "chipper-canvas";
/// Hidden canvas holding the EXIF-rotated photo, before cropping.
const SOURCE_ID: &str = "chipper-source";
/// Longest side kept from the photo, so a crop of a small region stays sharp.
const SOURCE_MAX: u32 = 4000;
const MAX_SIDE: u32 = 2000;
/// Small photos are upscaled so receipt text is tall enough for ocrs.
const MIN_SIDE: u32 = 1600;
const MAX_UPSCALE: f64 = 3.0;

/// Scale factor applied to the (cropped) image before OCR. The native OCR
/// tests use it too.
#[allow(dead_code)]
pub fn target_scale(w: u32, h: u32) -> f64 {
    let long = w.max(h).max(1) as f64;
    if long > MAX_SIDE as f64 {
        MAX_SIDE as f64 / long
    } else if long < MIN_SIDE as f64 {
        (MIN_SIDE as f64 / long).min(MAX_UPSCALE)
    } else {
        1.0
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LoadedImage {
    /// Object URL of the chosen file, for the preview and crop view.
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Deserialize)]
struct LoadReply {
    #[serde(default)]
    error: Option<String>,
    #[serde(flatten)]
    image: Option<LoadedImage>,
}

/// Decode the file selected in `<input id=input_id>` onto the source canvas.
pub async fn load_image(input_id: &str) -> Result<LoadedImage, String> {
    let js = format!(
        r##"
        const input = document.getElementById({input_id:?});
        const file = input && input.files && input.files[0];
        if (!file) {{
            dioxus.send({{ error: "No file selected" }});
        }} else {{
            try {{
                const bmp = await createImageBitmap(file, {{ imageOrientation: "from-image" }});
                const s = Math.min(1, {SOURCE_MAX} / Math.max(bmp.width, bmp.height));
                const w = Math.round(bmp.width * s), h = Math.round(bmp.height * s);
                let c = document.getElementById({SOURCE_ID:?});
                if (!c) {{
                    c = document.createElement("canvas");
                    c.id = {SOURCE_ID:?};
                    c.style.display = "none";
                    document.body.appendChild(c);
                }}
                c.width = w; c.height = h;
                const ctx = c.getContext("2d", {{ willReadFrequently: true }});
                ctx.imageSmoothingEnabled = true;
                ctx.imageSmoothingQuality = "high";
                ctx.fillStyle = "#fff";
                ctx.fillRect(0, 0, w, h);
                ctx.drawImage(bmp, 0, 0, w, h);
                bmp.close();
                input.value = "";
                if (window.__chipperUrl) URL.revokeObjectURL(window.__chipperUrl);
                window.__chipperUrl = URL.createObjectURL(file);
                dioxus.send({{ url: window.__chipperUrl, width: w, height: h }});
            }} catch (e) {{
                input.value = "";
                dioxus.send({{ error: "Couldn't read that image (" + e + "). Try a JPEG or PNG." }});
            }}
        }}
        "##
    );
    let mut ev = document::eval(&js);
    let reply: LoadReply = ev.recv().await.map_err(|e| format!("{e:?}"))?;
    match (reply.error, reply.image) {
        (Some(e), _) => Err(e),
        (None, Some(img)) => Ok(img),
        _ => Err("Unexpected reply while reading image".into()),
    }
}

/// Flatten `quad` of the source photo onto the OCR canvas, scaled for OCR.
#[cfg(target_arch = "wasm32")]
pub fn apply_crop(quad: &crate::warp::Quad) -> Result<(), String> {
    use wasm_bindgen::{Clamped, JsCast};
    let err = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let doc = web_sys::window().and_then(|w| w.document()).ok_or("no document")?;
    let canvas = |id: &str| -> Result<(web_sys::HtmlCanvasElement, web_sys::CanvasRenderingContext2d), String> {
        let c: web_sys::HtmlCanvasElement = match doc.get_element_by_id(id) {
            Some(el) => el.dyn_into().map_err(|_| "not a canvas")?,
            None => {
                let c: web_sys::HtmlCanvasElement = doc.create_element("canvas").map_err(err)?.dyn_into().map_err(|_| "not a canvas")?;
                c.set_id(id);
                c.set_attribute("style", "display:none").map_err(err)?;
                doc.body().ok_or("no body")?.append_child(&c).map_err(err)?;
                c
            }
        };
        let opts = js_sys::Object::new();
        js_sys::Reflect::set(&opts, &"willReadFrequently".into(), &true.into()).map_err(err)?;
        let ctx = c
            .get_context_with_context_options("2d", &opts)
            .map_err(err)?
            .ok_or("no 2d context")?
            .dyn_into()
            .map_err(|_| "not a 2d context")?;
        Ok((c, ctx))
    };
    let (src, sctx) = canvas(SOURCE_ID)?;
    let (sw, sh) = (src.width(), src.height());
    let (nw, nh) = crate::warp::natural_size(quad, sw, sh);
    let s = target_scale(nw.round() as u32, nh.round() as u32);
    let (ow, oh) = (((nw * s).round() as u32).max(1), ((nh * s).round() as u32).max(1));
    let (dst, dctx) = canvas(CANVAS_ID)?;
    dst.set_width(ow);
    dst.set_height(oh);
    if crate::warp::is_full(quad) {
        // Plain resize: let the browser's high-quality scaler do it.
        dctx.set_image_smoothing_enabled(true);
        js_sys::Reflect::set(&dctx, &"imageSmoothingQuality".into(), &"high".into()).map_err(err)?;
        return dctx
            .draw_image_with_html_canvas_element_and_dw_and_dh(&src, 0.0, 0.0, ow as f64, oh as f64)
            .map_err(err);
    }
    let pixels = sctx.get_image_data(0.0, 0.0, sw as f64, sh as f64).map_err(err)?.data().0;
    let out = crate::warp::warp(&pixels, sw, sh, quad, ow, oh);
    let img = web_sys::ImageData::new_with_u8_clamped_array_and_sh(Clamped(&out), ow, oh).map_err(err)?;
    dctx.put_image_data(&img, 0.0, 0.0).map_err(err)
}

pub fn copy_to_clipboard(text: &str) {
    let js = format!("navigator.clipboard && navigator.clipboard.writeText({});", serde_json::to_string(text).unwrap());
    document::eval(&js);
}
