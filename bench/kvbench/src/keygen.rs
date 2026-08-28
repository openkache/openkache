//! Deterministic, allocation-light key/value generation.
//!
//! Keys are a fixed 32 bytes: an 8-char ASCII prefix + 24-digit zero-padded
//! index. Values are `value_len` bytes derived deterministically from the key
//! index, so prefill and any later verification agree without storing anything.

pub const KEY_LEN: usize = 32;
const PREFIX: &[u8; 8] = b"kvbench:";

/// Writes the 32-byte key for `index` into `out`.
#[inline]
pub fn write_key(index: u64, out: &mut [u8; KEY_LEN]) {
    out[..8].copy_from_slice(PREFIX);
    let mut n = index;
    // 24 decimal digits, zero-padded, filling bytes [8, 32).
    for slot in out[8..].iter_mut().rev() {
        *slot = b'0' + (n % 10) as u8;
        n /= 10;
    }
}

/// Fills `out` (len = value_len) with deterministic pseudo-random bytes seeded
/// by `index`. Incompressible enough that stores can't cheat via compression.
#[inline]
pub fn write_value(index: u64, out: &mut [u8]) {
    let mut state = index.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
    for byte in out.iter_mut() {
        // xorshift64* step
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        *byte = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8;
    }
}

/// Fast per-connection RNG for picking random key indices during GET.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Avoid the all-zero state.
        Self(seed | 1)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform index in [0, n).
    #[inline]
    pub fn index(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}
