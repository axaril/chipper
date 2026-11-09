# Chipper

Split a receipt between friends. A static Dioxus/WebAssembly app built from the
"Budget Splitting" spreadsheet: snap a receipt, tap who had what, and tax & tip
are spread proportionally (each person's items × gross total ÷ subtotal), with
cents allocated so the shares always add up exactly.

Receipts are read **on-device** with [ocrs](https://github.com/robertknight/ocrs);
nothing is uploaded.

## Develop

```sh
cargo install dioxus-cli@0.7.10 --locked
dx serve                                   # http://127.0.0.1:8080
```

## Build the static site

```sh
dx bundle --platform web --release
# → target/dx/chipper/release/web/public  (serve or upload this folder)
```

## Deploy

Pushing to `main` runs [.github/workflows/deploy.yml](.github/workflows/deploy.yml):
tests (including OCR over `tests/`), a `dx bundle` with `--base-path <repo>`, then
a GitHub Pages deploy. One-time setup: in the repo's **Settings → Pages**, set
**Source** to **GitHub Actions**. The site is then at
`https://<user>.github.io/<repo>/`.

## Test

```sh
cargo test                                               # math, parser, price expressions
cargo test --release ocr_samples -- --ignored --nocapture  # OCR on the photos in tests/
```

## Layout

| File | What |
| --- | --- |
| `src/model.rs` | Bill, people, items, the split math (matches the spreadsheet) |
| `src/expr.rs` | Price cells accept expressions like `12.49-2.5` |
| `src/receipt.rs` | OCR text → line items, discounts, subtotal/tax/total |
| `src/ocr.rs` | ocrs engine, tilt-corrected row merging, browser glue |
| `src/scan.rs` | Image loading onto a canvas (EXIF rotation, resizing) |
| `assets/models/` | ocrs detection/recognition models (≈12 MB, fetched on first scan) |
| `assets/dots.js`, `assets/theme.js` | Dot-grid background and theme toggle from axaril.github.io |
