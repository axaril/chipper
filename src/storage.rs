//! Persist the bill in localStorage.

use crate::model::Bill;

const BILL_KEY: &str = "chipper.bill.v1";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn load_bill() -> Bill {
    storage()
        .and_then(|s| s.get_item(BILL_KEY).ok().flatten())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn save_bill(b: &Bill) {
    if let (Some(s), Ok(json)) = (storage(), serde_json::to_string(b)) {
        let _ = s.set_item(BILL_KEY, &json);
    }
}
