//! Application behavior for the route-less example APIs.
//!
//! This module deliberately has no generated-contract, wire-codec, transport,
//! client, or storage imports. The surrounding API bindings translate between
//! typed values and generated field envelopes, just as an independently owned
//! API module would.

pub(super) struct DenseValue {
    pub(super) counter: u64,
    pub(super) enabled: bool,
}

pub(super) struct Page {
    pub(super) items: Vec<Vec<u8>>,
    pub(super) next_cursor: Option<Vec<u8>>,
}

pub(super) const fn ping() -> &'static [u8] {
    b"PONG"
}

pub(super) fn echo<T>(value: T) -> T {
    value
}

pub(super) fn acknowledge(_token: &str) {}

pub(super) fn dense(value: DenseValue) -> DenseValue {
    value
}

pub(super) fn reverse(value: &str) -> String {
    value.chars().rev().collect()
}

pub(super) fn square(value: f64) -> Option<f64> {
    let squared = value * value;
    squared.is_finite().then_some(squared)
}

pub(super) fn page(cursor: Option<&[u8]>) -> Page {
    match cursor {
        None => Page {
            items: vec![b"first".to_vec(), b"second".to_vec()],
            next_cursor: Some(b"next".to_vec()),
        },
        Some([]) => Page {
            items: Vec::new(),
            next_cursor: Some(Vec::new()),
        },
        Some(_) => Page {
            items: vec![b"last".to_vec()],
            next_cursor: None,
        },
    }
}
