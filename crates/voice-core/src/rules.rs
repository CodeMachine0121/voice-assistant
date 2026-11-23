//! Business thresholds, in one place.
use std::time::Duration;

/// Presses shorter than this are accidental. A press of exactly this length counts.
pub const MIN_DICTATION: Duration = Duration::from_millis(500);
/// A dictation stops on its own when it reaches this length.
pub const MAX_DICTATION: Duration = Duration::from_secs(5 * 60);
/// The writing assistant must answer within this time or cleanup is skipped.
pub const ASSISTANT_TIMEOUT: Duration = Duration::from_secs(20);
/// Maximum number of history entries kept.
pub const HISTORY_LIMIT: usize = 200;
/// Sample rate every `Recording` is delivered in (what the speech model expects).
pub const SAMPLE_RATE: u32 = 16_000;
/// Edit selection accepts a selection of at most this many characters (inclusive).
pub const MAX_SELECTION_CHARS: usize = 10_000;
