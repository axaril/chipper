//! Turning raw receipt text (from OCR) into line items.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ParsedItem {
    pub name: String,
    pub price: f64,
    /// Positive amount taken off `price` (coupons, instant savings, ...).
    #[serde(default)]
    pub discount: f64,
}

impl ParsedItem {
    /// Spreadsheet-style price expression, e.g. `12.49-2.5`.
    pub fn price_expr(&self) -> String {
        if self.discount.abs() > 0.004 {
            format!("{}-{}", fmt_num(self.price), fmt_num(self.discount))
        } else {
            fmt_num(self.price)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ParsedReceipt {
    #[serde(default)]
    pub merchant: Option<String>,
    pub items: Vec<ParsedItem>,
    #[serde(default)]
    pub subtotal: Option<f64>,
    #[serde(default)]
    pub tax: Option<f64>,
    #[serde(default)]
    pub tip: Option<f64>,
    #[serde(default)]
    pub total: Option<f64>,
}

pub fn fmt_num(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".into() } else { s.to_string() }
}

/// `... 12.49 N` / `... 2.50-` / `... -$2.50` / OCR-mangled `6 99`, `.94`
/// at the end of a line (optionally followed by a tax flag or a stray char).
static PRICE_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?P<head>.*?)[\s:]*(?P<neg1>-)?\s*\$?\s*(?P<amt>\d{1,5}(?:,\d{3})*[.,:]\s?\d{2}|[.,]\d{2}|\d{1,3}\s\d{2})\s*(?P<neg2>-)?(?:\s*[A-Z*.~,^]{1,3}|\s+\S)?\s*$",
    )
    .unwrap()
});
/// Totals lines with trailing noise: `SUBTOTAL   118.89   OVEN`.
static FIRST_AMOUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<head>.*?)\s*\$?(?P<amt>\d{1,5}[.,:]\s?\d{2})(?:\s|$)").unwrap());
/// Totals lines where OCR dropped the decimal point: `SUBTOTAL 15713`.
static NODEC_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<head>.*?)\s*[^\d\s]?(?P<amt>\d{3,7})\s*$").unwrap());
static QTY_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*\d+(?:\.\d+)?\s*(?:@|x|ea\b|lb\b|/lb)").unwrap());
/// Optional one-letter flag (`E`) then one or more item codes.
static LEADING_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\S{1,2}\s+)?(?:[A-Z]\s*)?(?:[\d?]{3,}\s*)+").unwrap());
static DIGIT_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{4,}").unwrap());
static DISCOUNT_REF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/\s*(\d{3,})").unwrap());
static LETTERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z]{2,}").unwrap());
static WORD3: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z]{3,}").unwrap());

const SKIP_WORDS: &[&str] = &[
    "change", "cash", "visa", "mastercard", "amex", "discover", "debit", "deb1", "credit", "tender",
    "balance", "approved", "auth", "card", "payment", "chip", "account", "acct", "ref", "amount",
    "you saved", "items sold", "number of items", "points", "rewards", "member", "invoice",
    "thank", "phone", "tel", "receipt", "cashier", "register", "trans", "aid:", "tvr", "eft",
    "merchant",
];

#[derive(PartialEq)]
enum Kind {
    Subtotal,
    Tax,
    Tip,
    Total,
    Other,
}

fn classify(text: &str) -> Kind {
    let c: String = text.to_lowercase().chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if c.contains("subtotal") || c.contains("subtota") || c.contains("ubtotal") {
        Kind::Subtotal
    } else if (c.contains("tax") && !c.contains("taxable")) || (c.len() <= 4 && (c.starts_with("ta") || c.ends_with("ax"))) {
        Kind::Tax
    } else if c.contains("tip") || c.contains("gratuity") {
        Kind::Tip
    } else if c.contains("total") && !c.contains("saving") && !c.contains("item") && !c.contains("number") {
        Kind::Total
    } else {
        Kind::Other
    }
}

/// Heuristic parser for OCR'd receipt text. Works best on grocery /
/// warehouse-club style receipts where each item line ends with a price.
pub fn parse_text(text: &str) -> ParsedReceipt {
    let mut out = ParsedReceipt::default();
    // (item code, index into out.items)
    let mut codes: Vec<(String, usize)> = Vec::new();
    let mut pending_name: Option<String> = None;
    // Past the first SUBTOTAL/TOTAL: no more line items.
    let mut in_totals = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        let caps = PRICE_TAIL.captures(line);
        let kind = classify(caps.as_ref().map_or(line, |c| c.name("head").unwrap().as_str()));

        if out.merchant.is_none() && out.items.is_empty() && caps.is_none() && kind == Kind::Other && WORD3.is_match(line) {
            out.merchant = Some(line.to_string());
            continue;
        }

        let parsed = match &caps {
            Some(c) => normalize_amount(&c["amt"])
                .parse::<f64>()
                .ok()
                .map(|a| (c["head"].trim().to_string(), a, c.name("neg1").is_some() || c.name("neg2").is_some())),
            // Decimal point lost on a totals line.
            None if kind != Kind::Other => FIRST_AMOUNT
                .captures(line)
                .and_then(|c| normalize_amount(&c["amt"]).parse::<f64>().ok().map(|a| (c["head"].trim().to_string(), a)))
                .or_else(|| {
                    NODEC_TAIL
                        .captures(line)
                        .and_then(|c| c["amt"].parse::<f64>().ok().map(|a| (c["head"].trim().to_string(), a / 100.0)))
                })
                .map(|(h, a)| (h, a, false)),
            None => None,
        };

        let Some((head, amount, negative)) = parsed else {
            if matches!(kind, Kind::Subtotal | Kind::Total) {
                in_totals = true;
            } else if !in_totals && LETTERS.is_match(line) && !SKIP_WORDS.iter().any(|w| lower.contains(w)) {
                // Maybe the name of an item whose price is on the next line.
                pending_name = Some(clean_name(line));
            }
            continue;
        };

        match kind {
            Kind::Subtotal => {
                out.subtotal = Some(amount);
                in_totals = true;
                continue;
            }
            // Later tax lines ("TOTAL TAX") restate rather than add.
            Kind::Tax => {
                out.tax = Some(amount);
                continue;
            }
            Kind::Tip => {
                out.tip = Some(amount);
                continue;
            }
            Kind::Total => {
                out.total = Some(amount);
                in_totals = true;
                continue;
            }
            Kind::Other => {}
        }
        let head_lower = head.to_lowercase();
        let compact: String = head_lower.chars().filter(|c| c.is_alphabetic()).collect();
        if in_totals || SKIP_WORDS.iter().any(|w| head_lower.contains(w)) || (compact.contains("saving") && !negative) {
            continue;
        }

        // Costco-style `0000361 / 1460530  2.50-`: only codes, no words.
        let bare = head.trim_start_matches(|c: char| c.is_ascii_uppercase()).trim();
        let code_only = !LETTERS.is_match(bare) && DIGIT_RUN.find_iter(bare).count() >= 2;
        let is_discount = negative
            || code_only
            || head_lower.contains("coupon")
            || head_lower.contains("discount")
            || head_lower.contains("instant sav")
            || (DISCOUNT_REF.is_match(&head) && !LETTERS.is_match(&DISCOUNT_REF.replace(&head, "")));

        if is_discount {
            let referenced = DISCOUNT_REF
                .captures(&head)
                .map(|c| c[1].to_string())
                .or_else(|| DIGIT_RUN.find_iter(bare).last().filter(|_| code_only).map(|m| m.as_str().to_string()));
            let target = referenced
                .and_then(|r| codes.iter().rev().find(|(code, _)| *code == r).map(|(_, i)| *i))
                .or_else(|| out.items.len().checked_sub(1));
            if let Some(i) = target {
                out.items[i].discount += amount;
            }
            continue;
        }

        if QTY_LINE.is_match(&head) && !LETTERS.is_match(&QTY_LINE.replace(&head, "")) {
            // "2 @ 3.99   7.98" — belongs to the name on the previous line.
            if let Some(name) = pending_name.take() {
                out.items.push(ParsedItem { name, price: amount, discount: 0.0 });
            }
            continue;
        }

        let mut name = clean_name(&head);
        if !LETTERS.is_match(&name) {
            match pending_name.take() {
                Some(n) => name = n,
                None => continue,
            }
        }
        pending_name = None;
        if let Some(code) = LEADING_CODE.find(&head).and_then(|m| DIGIT_RUN.find_iter(m.as_str()).last()) {
            codes.push((code.as_str().to_string(), out.items.len()));
        }
        out.items.push(ParsedItem { name, price: amount, discount: 0.0 });
    }

    // SUBTOTAL + TAX (+ TIP) should equal TOTAL. When OCR garbled one side,
    // trust the parts (the TOTAL line is often printed highlighted and reads
    // worst) unless the subtotal is wildly off from the items but the total
    // fits them, e.g. a lost digit: `SUBTOTAL 856` for 85.61.
    let extras = out.tax.unwrap_or(0.0) + out.tip.unwrap_or(0.0);
    let items: f64 = out.items.iter().map(|i| i.price - i.discount).sum();
    let near_items = |v: f64| items > 0.0 && (v - items).abs() <= 0.1 * items;
    let round = |v: f64| (v * 100.0).round() / 100.0;
    match (out.subtotal, out.total) {
        (Some(st), Some(t)) if (st + extras - t).abs() > 0.011 => {
            if !near_items(st) && near_items(t - extras) {
                out.subtotal = Some(round(t - extras));
            } else {
                out.total = Some(round(st + extras));
            }
        }
        (Some(st), None) => out.total = Some(round(st + extras)),
        _ => {}
    }
    out
}

/// `1,234.50` / `3,49` / `12. 99` / `6 99` / `.94` →
/// `1234.50` / `3.49` / `12.99` / `6.99` / `0.94`.
fn normalize_amount(original: &str) -> String {
    let s = original.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.replace(':', ".");
    if !s.contains(['.', ',']) {
        return s.replace(' ', ".");
    }
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    // Last separator is the decimal point; earlier ones are thousands separators.
    let pos = s.rfind(['.', ',']).unwrap();
    let (int, frac) = s.split_at(pos);
    let int: String = int.chars().filter(|c| c.is_ascii_digit()).collect();
    format!("{}.{}", if int.is_empty() { "0" } else { &int }, &frac[1..])
}

fn clean_name(s: &str) -> String {
    let s = LEADING_CODE.replace(s.trim(), "");
    let s = s.trim_matches(|c: char| !c.is_alphanumeric() && c != ')' && c != '%');
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_costco_style() {
        let text = "COSTCO WHOLESALE
        E 1234567 CASCADE GEL 12.49 N
        0000362 / 1234567 2.50-
        E 7654321 TOFU 6.99 N
        512345 KS ORG EVOO 15.59 N
        E 99887 MUSHROOM 5.99
        SUBTOTAL 132.67
        TAX 6.19
        **** TOTAL 138.86
        VISA 138.86
        TOTAL NUMBER OF ITEMS SOLD = 13
        INSTANT SAVINGS $27.20";
        let r = parse_text(text);
        assert_eq!(r.merchant.as_deref(), Some("COSTCO WHOLESALE"));
        assert_eq!(r.items.len(), 4, "{:#?}", r.items);
        assert_eq!(r.items[0].name, "CASCADE GEL");
        assert_eq!(r.items[0].price_expr(), "12.49-2.5");
        assert_eq!(r.items[1].name, "TOFU");
        assert_eq!(r.items[2].name, "KS ORG EVOO");
        assert_eq!(r.items[3].price_expr(), "5.99");
        assert_eq!(r.subtotal, Some(132.67));
        assert_eq!(r.tax, Some(6.19));
        assert_eq!(r.total, Some(138.86));
    }

    #[test]
    fn parses_restaurant_style() {
        let text = "Luigi's Trattoria
        2 Margherita Pizza $28.00
        Caesar Salad $11.50
        Sparkling Water
        2 @ 3.00 6.00
        Coupon -5.00
        Sub Total: $45.50
        Sales Tax: $4.04
        Tip: $9.00
        Total: $58.54";
        let r = parse_text(text);
        let names: Vec<_> = r.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["2 Margherita Pizza", "Caesar Salad", "Sparkling Water"]);
        assert_eq!(r.items[2].price_expr(), "6-5");
        assert_eq!(r.subtotal, Some(45.5));
        assert_eq!(r.tax, Some(4.04));
        assert_eq!(r.tip, Some(9.0));
        assert_eq!(r.total, Some(58.54));
    }

    #[test]
    fn decimal_comma_and_spaces() {
        let r = parse_text("Shop\nBread 3,49\nCheese 12. 99");
        assert_eq!(r.items[0].price, 3.49);
        assert_eq!(r.items[1].price, 12.99);
    }

    /// Real OCR output from `tests/costco-santee.jpg` (abridged), with typical
    /// misreads: lost slashes/decimal points, `TAK`, duplicated tax lines.
    #[test]
    fn tolerates_ocr_noise() {
        let text = "osTCO
        SELF-CHECKOUT
        17 Penber 111824165644
        1734925 KS CHARCOA.   18.99A
        E 1300000000 ? REDEMA VA   20A
        E1800192 BRNE  BRTTLE   8.59
        E 0000327783 1800192   3.00
        E993467 PUNPKIN FLAX   8.69
        E 0000329847 /993467   3.00-
        E 1268367 0G CHRY ALMD   6 99
        E 2619 ORG.BANANA   .94
        E 24311 VAR. MUFFIN   7.99   7
        SUBTOTAL   15713
        TAK   6.44
        #ENN TOTAL   \"16350
        EFT/Deb!   63.57
        CHANGE   0.00
        A775? TAX
        6.44
        TOTAL TAX   6.44
        INSTANT SAVINGS 16
        6.00";
        let r = parse_text(text);
        let got: Vec<(String, String)> = r.items.iter().map(|i| (i.name.clone(), i.price_expr())).collect();
        let want = [
            ("KS CHARCOA", "18.99"),
            ("BRNE BRTTLE", "8.59-3"),
            ("PUNPKIN FLAX", "8.69-3"),
            ("0G CHRY ALMD", "6.99"),
            ("ORG.BANANA", "0.94"),
            ("VAR. MUFFIN", "7.99"),
        ];
        let want: Vec<(String, String)> = want.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        assert_eq!(got, want);
        assert_eq!(r.subtotal, Some(157.13));
        assert_eq!(r.tax, Some(6.44));
        assert_eq!(r.total, Some(163.57));
    }
}
