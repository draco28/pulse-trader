//! Content-addressed `data_version` generation (audit C7).
//!
//! `data_version = sha256(canonical)[..16 hex]` over a stable canonical encoding
//! of `(pair, timeframe, schema_version, candles)`. The canonical bytes use the
//! exact same UTF-8 Decimal strings and i64-ms timestamps that land in the
//! Parquet columns, so the id is reproducible across runs and architectures
//! (NFR-2). Identical data ⇒ identical version; any candle-set or
//! `schema_version` change ⇒ a new version automatically.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::domain::{Candle, DataVersion, Pair, Timeframe};

/// Number of hex characters retained from the SHA-256 digest (64-bit id).
const VERSION_HEX_LEN: usize = 16;

/// Number of digest bytes that produce [`VERSION_HEX_LEN`] hex chars (2 per byte).
const VERSION_BYTE_LEN: usize = VERSION_HEX_LEN / 2;

/// Compute the content-hash `data_version` for a candle set.
///
/// Folds `(pair, timeframe, schema_version, candles)` into a single SHA-256
/// digest and truncates to [`VERSION_HEX_LEN`] hex characters.
pub(crate) fn content_version(
    pair: &Pair,
    tf: Timeframe,
    schema_version: u32,
    candles: &[Candle],
) -> DataVersion {
    let mut hasher = Sha256::new();
    feed_canonical(&mut hasher, pair, tf, schema_version, candles);
    let digest = hasher.finalize();

    let mut hex = String::with_capacity(VERSION_HEX_LEN);
    for byte in digest.iter().take(VERSION_BYTE_LEN) {
        // `write!` to a String is infallible; the result is discarded.
        let _ = write!(hex, "{byte:02x}");
    }
    DataVersion::new(hex)
}

/// Feed the stable canonical encoding of the inputs into `hasher`.
///
/// Field order is fixed and every field is length-delimited or fixed-width so
/// no two distinct inputs can collide via boundary ambiguity. Decimal values use
/// their exact UTF-8 string form (the same bytes written to Parquet).
fn feed_canonical(
    hasher: &mut Sha256,
    pair: &Pair,
    tf: Timeframe,
    schema_version: u32,
    candles: &[Candle],
) {
    feed_str(hasher, pair.as_str());
    feed_str(hasher, tf.binance_interval());
    hasher.update(schema_version.to_be_bytes());
    hasher.update((candles.len() as u64).to_be_bytes());
    for c in candles {
        hasher.update(c.open_time.to_be_bytes());
        hasher.update(c.close_time.to_be_bytes());
        feed_str(hasher, &c.open.to_string());
        feed_str(hasher, &c.high.to_string());
        feed_str(hasher, &c.low.to_string());
        feed_str(hasher, &c.close.to_string());
        feed_str(hasher, &c.volume.to_string());
        match &c.funding_rate {
            Some(f) => {
                hasher.update([1u8]);
                feed_str(hasher, &f.to_string());
            }
            None => hasher.update([0u8]),
        }
    }
}

/// Length-prefixed string feed: an 8-byte big-endian length then the UTF-8 bytes,
/// so concatenation is unambiguous.
fn feed_str(hasher: &mut Sha256, s: &str) {
    hasher.update((s.len() as u64).to_be_bytes());
    hasher.update(s.as_bytes());
}
