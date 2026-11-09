//! Browser glue: reading the chosen image onto a canvas, clipboard.

use dioxus::prelude::*;
use serde::Deserialize;

/// Hidden canvas holding the (downscaled, EXIF-rotated) receipt image.
pub const CANVAS_ID: &str = "chipper-canvas";
const MAX_SIDE: u32 = 2000;
/// Small photos are upscaled so receipt text is tall enough for ocrs.
const MIN_SIDE: u32 = 1600;
const MAX_UPSCALE: f64 = 3.0;

/// Scale factor applied to an image before OCR. The JS in [`load_image`]
/// applies the same rule; this copy lets the native tests mirror it.
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
    /// JPEG data URL, used for the preview thumbnail.
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

/// Decode the file selected in `<input id=input_id>` onto the hidden canvas.
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
                const long = Math.max(bmp.width, bmp.height);
                const s = long > {MAX_SIDE} ? {MAX_SIDE} / long
                        : long < {MIN_SIDE} ? Math.min({MIN_SIDE} / long, {MAX_UPSCALE:.1})
                        : 1;
                const w = Math.round(bmp.width * s), h = Math.round(bmp.height * s);
                let c = document.getElementById({CANVAS_ID:?});
                if (!c) {{
                    c = document.createElement("canvas");
                    c.id = {CANVAS_ID:?};
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
                input.value = "";
                dioxus.send({{ url: c.toDataURL("image/jpeg", 0.88), width: w, height: h }});
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

pub fn copy_to_clipboard(text: &str) {
    let js = format!("navigator.clipboard && navigator.clipboard.writeText({});", serde_json::to_string(text).unwrap());
    document::eval(&js);
}
