//! The bill: people, items, and the split math from "Budget Splitting.xlsx".
//!
//! Spreadsheet equivalents:
//! * `Shared` (col B)       = sum of the weights on an item row
//! * person item subtotal   = SUMPRODUCT(price, weight / shared)
//! * `After Tax + Tip`      = person subtotal * (gross total / item subtotal)

use crate::expr;
use crate::receipt::ParsedReceipt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PALETTE: &[&str] = &[
    "#0065d8", "#00a8b5", "#c2410c", "#7c3aed", "#15803d", "#be185d", "#a16207", "#475569",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub id: u32,
    pub name: String,
    pub color: usize,
}

impl Person {
    pub fn color(&self) -> &'static str {
        PALETTE[self.color % PALETTE.len()]
    }
    pub fn initials(&self) -> String {
        let mut it = self.name.split_whitespace().filter_map(|w| w.chars().next());
        match (it.next(), it.next()) {
            (Some(a), Some(b)) => format!("{a}{b}").to_uppercase(),
            (Some(_), None) => self.name.chars().take(2).collect::<String>().to_uppercase(),
            _ => "?".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u32,
    pub name: String,
    /// Price as typed, e.g. `12.49-2.5`.
    pub price_expr: String,
    /// person id → weight (1 = one share; 2 = double share, ...)
    #[serde(default)]
    pub shares: BTreeMap<u32, f64>,
}

impl Item {
    pub fn price(&self) -> f64 {
        expr::eval_or_zero(&self.price_expr)
    }
    pub fn weight(&self, person: u32) -> f64 {
        self.shares.get(&person).copied().unwrap_or(0.0)
    }
    pub fn total_weight(&self) -> f64 {
        self.shares.values().sum()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bill {
    #[serde(default)]
    pub title: String,
    pub people: Vec<Person>,
    pub items: Vec<Item>,
    #[serde(default)]
    pub tax_expr: String,
    #[serde(default)]
    pub tip_expr: String,
    #[serde(default)]
    pub next_id: u32,
}

impl Default for Bill {
    fn default() -> Self {
        Self::example()
    }
}

impl Bill {
    pub fn empty() -> Self {
        let mut b = Bill {
            title: String::new(),
            people: vec![],
            items: vec![],
            tax_expr: String::new(),
            tip_expr: String::new(),
            next_id: 1,
        };
        b.add_person("Me");
        b.add_person("Friend");
        b
    }

    /// Sample bill (same numbers as the original spreadsheet).
    pub fn example() -> Self {
        let mut b = Bill { title: "Grocery run".into(), ..Bill::empty() };
        b.people.clear();
        let alex = b.add_person("Alex");
        let sam = b.add_person("Sam");
        let rows: &[(&str, &str, &[u32])] = &[
            ("Dishwasher Gel", "12.49-2.5", &[alex, sam]),
            ("Tofu", "6.99-1.5", &[sam]),
            ("Dish Soap", "8.99-2.5", &[alex, sam]),
            ("Vitamins", "39.99-8", &[alex]),
            ("Basil Pesto", "9.99", &[alex]),
            ("Olive Oil", "15.59", &[alex, sam]),
            ("Onion Dip", "13.99-4.2", &[sam]),
            ("Mushrooms", "5.99", &[alex, sam]),
            ("Romaine", "5.49", &[sam]),
            ("Spinach", "3.99", &[alex, sam]),
            ("Spicy Chicken", "14.59-4", &[sam]),
            ("Socks", "13.99", &[alex]),
            ("Eggs", "3.29", &[alex, sam]),
        ];
        for (name, price, who) in rows {
            let id = b.add_item(name, price);
            let item = b.items.iter_mut().find(|i| i.id == id).unwrap();
            for p in *who {
                item.shares.insert(*p, 1.0);
            }
        }
        b.tax_expr = "6.19".into();
        b
    }

    fn alloc_id(&mut self) -> u32 {
        self.next_id = self.next_id.max(1);
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn add_person(&mut self, name: &str) -> u32 {
        let id = self.alloc_id();
        let used: Vec<usize> = self.people.iter().map(|p| p.color).collect();
        let color = (0..PALETTE.len()).find(|c| !used.contains(c)).unwrap_or(self.people.len());
        self.people.push(Person { id, name: name.to_string(), color });
        id
    }

    pub fn remove_person(&mut self, id: u32) {
        self.people.retain(|p| p.id != id);
        for item in &mut self.items {
            item.shares.remove(&id);
        }
    }

    pub fn add_item(&mut self, name: &str, price_expr: &str) -> u32 {
        let id = self.alloc_id();
        self.items.push(Item { id, name: name.into(), price_expr: price_expr.into(), shares: BTreeMap::new() });
        id
    }

    pub fn item_mut(&mut self, id: u32) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }

    pub fn apply_receipt(&mut self, r: &ParsedReceipt, replace: bool) {
        if replace {
            self.items.clear();
            self.tax_expr.clear();
            self.tip_expr.clear();
            if let Some(m) = &r.merchant {
                self.title = m.clone();
            }
        }
        for it in &r.items {
            self.add_item(&it.name, &it.price_expr());
        }
        // Amounts are added onto whatever is already there (append mode).
        let add = |expr: &mut String, v: f64| {
            if v.abs() > 0.004 {
                let v = crate::receipt::fmt_num(v);
                *expr = if expr.trim().is_empty() { v } else { format!("{expr}+{v}").replace("+-", "-") };
            }
        };
        let items: f64 = r.items.iter().map(|i| i.price - i.discount).sum();
        // Only a grand total printed: the difference is most likely tax.
        let tax = r.tax.unwrap_or_else(|| match (r.tip, r.total) {
            (None, Some(total)) if total > items => total - items,
            _ => 0.0,
        });
        let mut tip = r.tip.unwrap_or(0.0);
        // Make the bill add up to what was actually paid (fuel/member discounts
        // printed after the subtotal, or OCR misreads of item prices); the
        // difference is shared proportionally like tax and tip.
        if let Some(total) = r.total {
            tip += total - items - tax - tip;
        }
        add(&mut self.tax_expr, tax);
        add(&mut self.tip_expr, tip);
    }

    pub fn subtotal(&self) -> f64 {
        self.items.iter().map(Item::price).sum()
    }
    pub fn tax(&self) -> f64 {
        expr::eval_or_zero(&self.tax_expr)
    }
    pub fn tip(&self) -> f64 {
        expr::eval_or_zero(&self.tip_expr)
    }
    pub fn total(&self) -> f64 {
        self.subtotal() + self.tax() + self.tip()
    }

    pub fn summary(&self) -> Summary {
        let subtotal = self.subtotal();
        let total = self.total();
        let factor = if subtotal.abs() > 1e-9 { total / subtotal } else { 1.0 };

        let mut people: Vec<PersonShare> = self
            .people
            .iter()
            .map(|p| PersonShare { person: p.clone(), subtotal: 0.0, exact_total: 0.0, cents: 0, items: vec![] })
            .collect();
        let mut unassigned = vec![];
        let mut unassigned_amount = 0.0;

        for item in &self.items {
            let w = item.total_weight();
            let price = item.price();
            if w <= 0.0 {
                unassigned.push(item.id);
                unassigned_amount += price;
                continue;
            }
            for share in &mut people {
                let pw = item.weight(share.person.id);
                if pw > 0.0 {
                    let portion = price * pw / w;
                    share.subtotal += portion;
                    share.items.push(ItemPortion { name: item.name.clone(), portion, weight: pw, of: w });
                }
            }
        }
        for s in &mut people {
            s.exact_total = s.subtotal * factor;
        }
        allocate_cents(&mut people);

        Summary { subtotal, tax: self.tax(), tip: self.tip(), total, factor, unassigned, unassigned_amount, people }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemPortion {
    pub name: String,
    pub portion: f64,
    pub weight: f64,
    pub of: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PersonShare {
    pub person: Person,
    pub subtotal: f64,
    pub exact_total: f64,
    /// Rounded so that all shares add up to exactly the rounded total.
    pub cents: i64,
    pub items: Vec<ItemPortion>,
}

impl PersonShare {
    pub fn amount(&self) -> f64 {
        self.cents as f64 / 100.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub subtotal: f64,
    pub tax: f64,
    pub tip: f64,
    pub total: f64,
    /// gross / subtotal, the sheet's "C2/C3"
    pub factor: f64,
    pub unassigned: Vec<u32>,
    pub unassigned_amount: f64,
    pub people: Vec<PersonShare>,
}

impl Summary {
    pub fn assigned_cents(&self) -> i64 {
        self.people.iter().map(|p| p.cents).sum()
    }
}

/// Largest-remainder rounding: nobody is off by more than a cent and the
/// rounded shares sum to the rounded total.
fn allocate_cents(shares: &mut [PersonShare]) {
    let target = (shares.iter().map(|s| s.exact_total).sum::<f64>() * 100.0).round() as i64;
    let mut floors: Vec<(usize, f64)> = Vec::with_capacity(shares.len());
    let mut sum = 0;
    for (i, s) in shares.iter_mut().enumerate() {
        let c = s.exact_total * 100.0;
        s.cents = c.floor() as i64;
        sum += s.cents;
        floors.push((i, c - c.floor()));
    }
    floors.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (i, _) in floors.into_iter().take((target - sum).max(0) as usize) {
        shares[i].cents += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_spreadsheet() {
        let b = Bill::example();
        let s = b.summary();
        assert!((s.subtotal - 132.67).abs() < 1e-9);
        assert!((s.total - 138.86).abs() < 1e-9);
        let an = &s.people[0];
        let pat = &s.people[1];
        assert!((an.subtotal - 78.64).abs() < 1e-9, "{}", an.subtotal);
        assert!((pat.subtotal - 54.03).abs() < 1e-9);
        assert!((an.exact_total - 82.30911585136053).abs() < 1e-9);
        assert!((pat.exact_total - 56.55088414863948).abs() < 1e-9);
        assert_eq!(an.cents + pat.cents, 13886);
        assert_eq!(an.cents, 8231);
        assert_eq!(an.items.len(), 9);
        assert_eq!(pat.items.len(), 10);
    }

    #[test]
    fn receipt_total_wins() {
        let r = ParsedReceipt {
            items: vec![
                crate::receipt::ParsedItem { name: "A".into(), price: 50.0, discount: 0.0 },
                crate::receipt::ParsedItem { name: "B".into(), price: 35.17, discount: 0.0 },
            ],
            subtotal: Some(79.34),
            tax: Some(0.0),
            total: Some(79.34),
            ..Default::default()
        };
        let mut b = Bill::empty();
        b.apply_receipt(&r, true);
        assert!((b.total() - 79.34).abs() < 1e-9, "{}", b.total());
        assert_eq!(b.tip_expr, "-5.83");

        // Total only: difference becomes tax.
        let r = ParsedReceipt { total: Some(90.0), subtotal: None, tax: None, ..r };
        b.apply_receipt(&r, true);
        assert_eq!((b.tax_expr.as_str(), b.tip_expr.as_str()), ("4.83", ""));
    }

    #[test]
    fn splits_by_portions() {
        let mut b = Bill::empty();
        let (a, c) = (b.people[0].id, b.people[1].id);
        let id = b.add_item("Pizza", "10");
        let it = b.item_mut(id).unwrap();
        it.shares.insert(a, 1.0);
        it.shares.insert(c, 3.0);
        let s = b.summary();
        assert_eq!((s.people[0].cents, s.people[1].cents), (250, 750));
        let p = &s.people[1].items[0];
        assert_eq!((p.weight, p.of), (3.0, 4.0));

        // Fractional portions work too: 0.5 vs 1.5 is the same 1:3 ratio.
        let it = b.item_mut(id).unwrap();
        it.shares.insert(a, 0.5);
        it.shares.insert(c, 1.5);
        let s = b.summary();
        assert_eq!((s.people[0].cents, s.people[1].cents), (250, 750));
    }

    #[test]
    fn cents_always_add_up() {
        let mut b = Bill::empty();
        b.add_person("C");
        let id = b.add_item("Pizza", "10");
        let ids: Vec<u32> = b.people.iter().map(|p| p.id).collect();
        for p in ids {
            b.item_mut(id).unwrap().shares.insert(p, 1.0);
        }
        let s = b.summary();
        assert_eq!(s.assigned_cents(), 1000);
        let mut c: Vec<i64> = s.people.iter().map(|p| p.cents).collect();
        c.sort();
        assert_eq!(c, [333, 333, 334]);
    }
}
