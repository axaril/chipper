mod expr;
mod model;
mod ocr;
mod receipt;
mod scan;
mod storage;

use dioxus::prelude::*;
use model::{Bill, Summary};
use receipt::{ParsedReceipt, fmt_num};

const CSS: Asset = asset!("/assets/main.css");
const DOTS_JS: Asset = asset!("/assets/dots.js");
const THEME_JS: Asset = asset!("/assets/theme.js");
const FILE_INPUT_ID: &str = "receipt-file";

fn main() {
    dioxus::launch(App);
}

fn money(v: f64) -> String {
    if v < -0.0049 { format!("-${:.2}", -v) } else { format!("${:.2}", v.abs()) }
}

#[component]
fn App() -> Element {
    let bill = use_signal(storage::load_bill);
    use_effect(move || storage::save_bill(&bill.read()));
    let summary = use_memo(move || bill.read().summary());

    rsx! {
        document::Title { "Chipper – Split the bill" }
        document::Meta { name: "color-scheme", content: "light dark" }
        document::Stylesheet { href: CSS }
        document::Script { src: THEME_JS }
        document::Script { src: DOTS_JS, defer: true }
        canvas { id: "dots", "aria-hidden": "true" }
        div { class: "page",
            Header { bill }
            div { class: "layout",
                div { class: "col",
                    ScanPanel { bill }
                    ItemsPanel { bill, summary }
                }
                div { class: "col side",
                    PeoplePanel { bill }
                    TotalsPanel { bill, summary }
                    ResultsPanel { bill, summary }
                }
            }
            footer { class: "foot",
                "Runs entirely in your browser · receipts are read on-device with "
                a { href: "https://github.com/robertknight/ocrs", target: "_blank", "ocrs" }
            }
        }
    }
}

#[component]
fn ThemeToggle() -> Element {
    rsx! {
        button { r#type: "button", class: "theme-toggle", id: "theme-toggle", "aria-label": "Toggle colour theme",
            svg { class: "sun", view_box: "0 0 24 24", "aria-hidden": "true",
                circle { cx: "12", cy: "12", r: "4" }
                path { d: "M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" }
            }
            svg { class: "moon", view_box: "0 0 24 24", "aria-hidden": "true",
                path { d: "M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" }
            }
        }
    }
}

#[component]
fn Header(bill: Signal<Bill>) -> Element {
    let title = bill.read().title.clone();
    rsx! {
        div { class: "panel hero",
            ThemeToggle {}
            header { class: "band",
                h1 { "Chipper" }
                p { class: "tagline", "Scan a receipt, tap who had what, and split tax & tip fairly." }
            }
            div { class: "hero-body",
                label { class: "label", r#for: "bill-title", "Bill" }
                input {
                    id: "bill-title",
                    class: "bill-title",
                    placeholder: "Name this bill…",
                    value: "{title}",
                    oninput: move |e| bill.write().title = e.value(),
                }
                ul { class: "links",
                    li {
                        button {
                            onclick: move |_| {
                                let people: Vec<String> = bill.read().people.iter().map(|p| p.name.clone()).collect();
                                let mut b = Bill::empty();
                                if !people.is_empty() {
                                    b.people.clear();
                                    for p in &people {
                                        b.add_person(p);
                                    }
                                }
                                bill.set(b);
                            },
                            span { "New bill" }
                            span { class: "hint", "keeps people" }
                        }
                    }
                    li {
                        button { onclick: move |_| bill.set(Bill::example()),
                            span { "Load example" }
                            span { class: "hint", "sample bill" }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
enum ScanState {
    Idle,
    Working { progress: f32, label: String },
    Review(ParsedReceipt),
    Error(String),
}

#[component]
fn ScanPanel(bill: Signal<Bill>) -> Element {
    let mut state = use_signal(|| ScanState::Idle);
    let mut preview = use_signal(|| None::<String>);
    let mut raw_text = use_signal(String::new);
    let busy = matches!(*state.read(), ScanState::Working { .. });

    let on_file = move |_| {
        spawn(async move {
            state.set(ScanState::Working { progress: 0.01, label: "Reading image...".into() });
            let img = match scan::load_image(FILE_INPUT_ID).await {
                Ok(i) => i,
                Err(e) => return state.set(ScanState::Error(e)),
            };
            preview.set(Some(img.url));
            let result = run_ocrs(state).await.map(|text| {
                let r = receipt::parse_text(&text);
                raw_text.set(text);
                r
            });
            state.set(match result {
                Ok(r) if !r.items.is_empty() => ScanState::Review(r),
                Ok(_) => ScanState::Error(
                    "No line items found. Try a sharper, flatter photo, or fix the recognised text below and parse it again.".into(),
                ),
                Err(e) => ScanState::Error(e),
            });
        });
    };

    let status = match &*state.read() {
        ScanState::Idle => "Waiting for a receipt...".to_string(),
        ScanState::Working { label, .. } => label.clone(),
        ScanState::Review(r) => format!("Found {} items.", r.items.len()),
        ScanState::Error(_) => "Couldn't read that one.".to_string(),
    };
    let progress = match &*state.read() {
        ScanState::Working { progress, .. } => Some(*progress),
        _ => None,
    };

    rsx! {
        section { class: "panel",
            h2 { "Scan" }
            label { class: if busy { "dropzone busy" } else { "dropzone" },
                input {
                    id: FILE_INPUT_ID,
                    r#type: "file",
                    accept: "image/*",
                    disabled: busy,
                    onchange: on_file,
                }
                if let Some(url) = preview() {
                    img { class: "thumb", src: "{url}", alt: "Receipt preview" }
                } else {
                    div { class: "thumb empty", "aria-hidden": "true",
                        svg { view_box: "0 0 24 24",
                            path { d: "M6 2h12v20l-3-2-3 2-3-2-3 2zM9 7h6M9 11h6M9 15h4" }
                        }
                    }
                }
                div { class: "drop-text",
                    strong { if preview().is_some() { "Scan another receipt" } else { "Snap or choose a receipt" } }
                    span { "Read on this device — nothing is uploaded" }
                }
            }

            code { class: if busy { "status live" } else { "status" }, "{status}" }
            if let Some(p) = progress {
                div { class: "bar", div { class: "fill", style: "width: {p * 100.0:.0}%" } }
            }

            match state() {
                ScanState::Error(e) => rsx! {
                    p { class: "alert", "{e}" }
                },
                ScanState::Review(r) => rsx! {
                    ReviewPanel {
                        receipt: r.clone(),
                        has_items: !bill.read().items.is_empty(),
                        on_apply: move |replace: bool| {
                            bill.write().apply_receipt(&r, replace);
                            state.set(ScanState::Idle);
                        },
                        on_discard: move |_| state.set(ScanState::Idle),
                    }
                },
                _ => rsx! {},
            }

            details { class: "raw",
                summary { "Recognised text" }
                textarea {
                    rows: 8,
                    placeholder: "OCR output appears here. You can also paste or type receipt text and parse it.",
                    value: "{raw_text}",
                    oninput: move |e| raw_text.set(e.value()),
                }
                button {
                    class: "btn",
                    disabled: raw_text.read().trim().is_empty(),
                    onclick: move |_| {
                        let r = receipt::parse_text(&raw_text.read());
                        state.set(if r.items.is_empty() {
                            ScanState::Error("No priced lines found in that text.".into())
                        } else {
                            ScanState::Review(r)
                        });
                    },
                    "Parse text"
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn run_ocrs(mut state: Signal<ScanState>) -> Result<String, String> {
    ocr::recognize(move |progress, label| state.set(ScanState::Working { progress, label: label.into() })).await
}

#[cfg(not(target_arch = "wasm32"))]
async fn run_ocrs(_state: Signal<ScanState>) -> Result<String, String> {
    Err("On-device OCR is only available in the browser build.".into())
}

#[component]
fn ReviewPanel(receipt: ParsedReceipt, has_items: bool, on_apply: EventHandler<bool>, on_discard: EventHandler<()>) -> Element {
    let sum: f64 = receipt.items.iter().map(|i| i.price - i.discount).sum();
    let mismatch = receipt.subtotal.filter(|st| (st - sum).abs() > 0.015);
    rsx! {
        div { class: "review",
            ul { class: "entries",
                for (i , it) in receipt.items.iter().enumerate() {
                    li { key: "{i}",
                        span { class: "what", "{it.name}" }
                        span { class: "when",
                            if it.discount > 0.0 {
                                s { "{money(it.price)}" }
                                " "
                            }
                            "{money(it.price - it.discount)}"
                        }
                    }
                }
                li { class: "sum",
                    span { class: "what", "Items" }
                    span { class: "when", "{money(sum)}" }
                }
                if let Some(t) = receipt.tax {
                    li { span { class: "what", "Tax" } span { class: "when", "{money(t)}" } }
                }
                if let Some(t) = receipt.tip {
                    li { span { class: "what", "Tip" } span { class: "when", "{money(t)}" } }
                }
                if let Some(t) = receipt.total {
                    li { span { class: "what", "Total" } span { class: "when", "{money(t)}" } }
                }
            }
            if let Some(st) = mismatch {
                p { class: "alert",
                    "The receipt's subtotal is {money(st)} but these items add up to {money(sum)}. Some lines may be misread; you can fix them in the table."
                }
            }
            div { class: "actions",
                button { class: "btn primary", onclick: move |_| on_apply.call(true),
                    if has_items { "Replace items" } else { "Use these items" }
                }
                if has_items {
                    button { class: "btn", onclick: move |_| on_apply.call(false), "Add to items" }
                }
                button { class: "btn ghost", onclick: move |_| on_discard.call(()), "Discard" }
            }
        }
    }
}

#[component]
fn ItemsPanel(bill: Signal<Bill>, summary: Memo<Summary>) -> Element {
    let (people, items) = {
        let b = bill.read();
        (b.people.clone(), b.items.clone())
    };
    let unassigned = summary.read().unassigned.len();

    rsx! {
        section { class: "panel",
            h2 { "Who had what" }
            p { class: "hint-line", "Tap to assign. Right-click or long-press to give someone an extra share." }
            if items.is_empty() {
                p { class: "empty", "No items yet. Scan a receipt or add one by hand." }
            } else {
                div { class: "table-wrap",
                    table { class: "items",
                        thead {
                            tr {
                                th { class: "c-name", "Item" }
                                th { class: "c-price", "Price" }
                                for p in people.iter() {
                                    th { key: "{p.id}", class: "c-person",
                                        button {
                                            class: "tag",
                                            style: "--c: {p.color()}",
                                            title: "Toggle {p.name} on every item",
                                            onclick: {
                                                let pid = p.id;
                                                move |_| {
                                                    let mut b = bill.write();
                                                    let all = b.items.iter().all(|i| i.weight(pid) > 0.0);
                                                    for it in b.items.iter_mut() {
                                                        if all {
                                                            it.shares.remove(&pid);
                                                        } else {
                                                            it.shares.entry(pid).or_insert(1.0);
                                                        }
                                                    }
                                                }
                                            },
                                            "{p.initials()}"
                                        }
                                    }
                                }
                                th { class: "c-split", "Split" }
                                th { class: "c-del" }
                            }
                        }
                        tbody {
                            for item in items.iter() {
                                ItemRow { key: "{item.id}", bill, item_id: item.id }
                            }
                        }
                    }
                }
            }
            div { class: "actions",
                button { class: "btn", onclick: move |_| { bill.write().add_item("", ""); }, "+ Add item" }
                if unassigned > 0 {
                    button {
                        class: "btn warn",
                        onclick: move |_| {
                            let mut b = bill.write();
                            let ids: Vec<u32> = b.people.iter().map(|p| p.id).collect();
                            for it in b.items.iter_mut().filter(|i| i.total_weight() <= 0.0) {
                                for id in &ids {
                                    it.shares.insert(*id, 1.0);
                                }
                            }
                        },
                        "Split {unassigned} unassigned between everyone"
                    }
                }
            }
        }
    }
}

#[component]
fn ItemRow(bill: Signal<Bill>, item_id: u32) -> Element {
    let b = bill.read();
    let Some(item) = b.items.iter().find(|i| i.id == item_id).cloned() else {
        return rsx! {};
    };
    let people = b.people.clone();
    drop(b);

    let price = item.price();
    let valid = expr::eval(&item.price_expr).is_some() || item.price_expr.trim().is_empty();
    let is_expr = item.price_expr.trim_start_matches('-').contains(['+', '-', '*', '/', '(']);
    let w = item.total_weight();
    let everyone = !people.is_empty() && people.iter().all(|p| item.weight(p.id) > 0.0);

    let mut set_weight = move |pid: u32, f: fn(f64) -> f64| {
        if let Some(it) = bill.write().item_mut(item_id) {
            let nw = f(it.weight(pid));
            if nw > 0.0 {
                it.shares.insert(pid, nw);
            } else {
                it.shares.remove(&pid);
            }
        }
    };

    rsx! {
        tr { class: if w <= 0.0 { "unassigned" } else { "" },
            td { class: "c-name",
                input {
                    value: "{item.name}",
                    placeholder: "Item name",
                    oninput: move |e| if let Some(it) = bill.write().item_mut(item_id) { it.name = e.value() },
                }
            }
            td { class: "c-price",
                input {
                    class: if valid { "num" } else { "num invalid" },
                    value: "{item.price_expr}",
                    placeholder: "0.00",
                    inputmode: "decimal",
                    title: "Accepts expressions like 12.49-2.5",
                    oninput: move |e| if let Some(it) = bill.write().item_mut(item_id) { it.price_expr = e.value() },
                }
                if is_expr && valid {
                    div { class: "eval", "= {money(price)}" }
                }
            }
            for p in people.iter() {
                {
                    let pid = p.id;
                    let pw = item.weight(pid);
                    let portion = if w > 0.0 { price * pw / w } else { 0.0 };
                    rsx! {
                        td { key: "{pid}", class: "c-person",
                            button {
                                class: if pw > 0.0 { "cell on" } else { "cell" },
                                style: "--c: {p.color()}",
                                title: if pw > 0.0 { format!("{}: {}", p.name, money(portion)) } else { format!("Assign to {}", p.name) },
                                onclick: move |_| set_weight(pid, |w| if w > 0.0 { 0.0 } else { 1.0 }),
                                oncontextmenu: move |e| {
                                    e.prevent_default();
                                    set_weight(pid, |w| (w + 1.0).min(9.0));
                                },
                                if pw > 1.0 { "×{fmt_num(pw)}" } else if pw > 0.0 { "✓" } else { "" }
                            }
                        }
                    }
                }
            }
            td { class: "c-split",
                button {
                    class: if w <= 0.0 { "chip warn" } else if everyone { "chip on" } else { "chip" },
                    title: if everyone { "Clear" } else { "Split between everyone" },
                    onclick: move |_| {
                        let ids: Vec<u32> = bill.read().people.iter().map(|p| p.id).collect();
                        if let Some(it) = bill.write().item_mut(item_id) {
                            if everyone {
                                it.shares.clear();
                            } else {
                                for id in ids {
                                    it.shares.insert(id, 1.0);
                                }
                            }
                        }
                    },
                    if w <= 0.0 { "none" } else if everyone { "all" } else if item.shares.len() == 1 { "solo" } else { "÷{fmt_num(w)}" }
                }
            }
            td { class: "c-del",
                button {
                    class: "x",
                    title: "Remove item",
                    onclick: move |_| bill.write().items.retain(|i| i.id != item_id),
                    "✕"
                }
            }
        }
    }
}

#[component]
fn PeoplePanel(bill: Signal<Bill>) -> Element {
    let mut new_name = use_signal(String::new);
    let people = bill.read().people.clone();
    let mut add = move || {
        let name = new_name.read().trim().to_string();
        if !name.is_empty() {
            bill.write().add_person(&name);
            new_name.set(String::new());
        }
    };
    rsx! {
        section { class: "panel",
            h2 { "People" }
            ul { class: "entries people",
                for p in people.iter() {
                    li { key: "{p.id}", style: "--c: {p.color()}",
                        button {
                            class: "tag",
                            title: "Change colour",
                            onclick: {
                                let pid = p.id;
                                move |_| {
                                    if let Some(p) = bill.write().people.iter_mut().find(|x| x.id == pid) {
                                        p.color = (p.color + 1) % model::PALETTE.len();
                                    }
                                }
                            },
                            "{p.initials()}"
                        }
                        input {
                            value: "{p.name}",
                            "aria-label": "Name",
                            oninput: {
                                let pid = p.id;
                                move |e: FormEvent| {
                                    if let Some(p) = bill.write().people.iter_mut().find(|x| x.id == pid) {
                                        p.name = e.value();
                                    }
                                }
                            },
                        }
                        button {
                            class: "x",
                            title: "Remove {p.name}",
                            onclick: {
                                let pid = p.id;
                                move |_| bill.write().remove_person(pid)
                            },
                            "✕"
                        }
                    }
                }
                li { class: "add",
                    input {
                        placeholder: "Add a person…",
                        value: "{new_name}",
                        oninput: move |e| new_name.set(e.value()),
                        onkeydown: move |e| if e.key() == Key::Enter { add() },
                    }
                    button { class: "btn primary", onclick: move |_| add(), "Add" }
                }
            }
        }
    }
}

#[component]
fn TotalsPanel(bill: Signal<Bill>, summary: Memo<Summary>) -> Element {
    let s = summary();
    let (tax_expr, tip_expr) = {
        let b = bill.read();
        (b.tax_expr.clone(), b.tip_expr.clone())
    };
    let markup = (s.factor - 1.0) * 100.0;
    let total_str = format!("{:.2}", s.total);

    rsx! {
        section { class: "panel",
            h2 { "Tax & tip" }
            ul { class: "entries totals",
                li {
                    span { class: "what", "Items subtotal" }
                    span { class: "when", "{money(s.subtotal)}" }
                }
                li {
                    label { class: "what", r#for: "tax", "Tax" }
                    input {
                        id: "tax",
                        class: "num",
                        inputmode: "decimal",
                        placeholder: "0.00",
                        value: "{tax_expr}",
                        oninput: move |e| bill.write().tax_expr = e.value(),
                    }
                }
                li {
                    label { class: "what", r#for: "tip", "Tip / adjustments" }
                    input {
                        id: "tip",
                        class: "num",
                        inputmode: "decimal",
                        placeholder: "0.00",
                        value: "{tip_expr}",
                        oninput: move |e| bill.write().tip_expr = e.value(),
                    }
                }
                li { class: "chips",
                    for pct in [0u32, 15, 18, 20, 25] {
                        button {
                            key: "{pct}",
                            class: "chip",
                            onclick: move |_| {
                                let mut b = bill.write();
                                let pre_tax = b.subtotal();
                                b.tip_expr = if pct == 0 { String::new() } else { format!("{:.2}", pre_tax * pct as f64 / 100.0) };
                            },
                            if pct == 0 { "no tip" } else { "{pct}%" }
                        }
                    }
                }
                li { class: "grand",
                    label { class: "what", r#for: "total", "Total paid" }
                    input {
                        id: "total",
                        class: "num",
                        inputmode: "decimal",
                        title: "Type the receipt's grand total; the difference goes into tip / adjustments",
                        value: "{total_str}",
                        onchange: move |e| {
                            if let Some(t) = expr::eval(&e.value()) {
                                let mut b = bill.write();
                                let tip = t - b.subtotal() - b.tax();
                                b.tip_expr = if tip.abs() < 0.005 { String::new() } else { format!("{tip:.2}") };
                            }
                        },
                    }
                }
            }
            p { class: "hint-line", "Everyone's items are marked up by {markup:.2}% to cover tax & tip." }
        }
    }
}

#[component]
fn ResultsPanel(bill: Signal<Bill>, summary: Memo<Summary>) -> Element {
    let s = summary();
    let mut copied = use_signal(|| false);
    let total_assigned = s.assigned_cents() as f64 / 100.0;
    let title = bill.read().title.clone();

    let share_text = {
        let head = if title.trim().is_empty() { "Bill split".to_string() } else { title.clone() };
        let mut out = format!("{head} — total {}\n", money(s.total));
        for p in &s.people {
            let names: Vec<&str> = p.items.iter().map(|i| i.name.as_str()).collect();
            out.push_str(&format!("• {}: {}  ({})\n", p.person.name, money(p.amount()), names.join(", ")));
        }
        out
    };

    rsx! {
        section { class: "panel",
            h2 { "Who owes what" }
            if s.total > 0.0 {
                div { class: "stack",
                    for p in s.people.iter().filter(|p| p.cents > 0) {
                        div {
                            key: "{p.person.id}",
                            class: "seg",
                            style: "flex-grow: {p.exact_total.max(0.0)}; --c: {p.person.color()}",
                            title: "{p.person.name}: {money(p.amount())}",
                        }
                    }
                    if s.unassigned_amount > 0.0 {
                        div { class: "seg unassigned", style: "flex-grow: {s.unassigned_amount * s.factor}", title: "Unassigned" }
                    }
                }
            }
            ul { class: "entries shares",
                for p in s.people.iter() {
                    li { key: "{p.person.id}", style: "--c: {p.person.color()}",
                        details {
                            summary {
                                span { class: "what",
                                    span { class: "swatch" }
                                    "{p.person.name}"
                                    small { "{p.items.len()} items · {money(p.subtotal)} + {money(p.exact_total - p.subtotal)}" }
                                }
                                span { class: "amount", "{money(p.amount())}" }
                            }
                            ul { class: "share-items",
                                for (i , it) in p.items.iter().enumerate() {
                                    li { key: "{i}",
                                        span { "{it.name}" }
                                        if it.of > 1.0 || it.weight > 1.0 {
                                            span { class: "frac", "{fmt_num(it.weight)}/{fmt_num(it.of)}" }
                                        }
                                        span { class: "num", "{money(it.portion)}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if s.unassigned_amount > 0.0 {
                p { class: "alert",
                    "{s.unassigned.len()} item(s) worth {money(s.unassigned_amount)} aren't assigned to anyone yet."
                }
            } else if !s.people.is_empty() && s.total > 0.0 {
                p { class: "check", "Shares add up to {money(total_assigned)}" }
            }
            div { class: "actions",
                button {
                    class: "btn primary",
                    onclick: move |_| {
                        scan::copy_to_clipboard(&share_text);
                        copied.set(true);
                    },
                    if copied() { "Copied ✓" } else { "Copy summary" }
                }
            }
        }
    }
}
