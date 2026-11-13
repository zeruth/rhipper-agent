//! The one clock stamps are read from. A test pins it to a second of its choosing, so a stamp,
//! an age or a span it asserts does not roll over while the suite runs under load; the pin is
//! the test thread's own, so tests running beside it keep the real time.

use std::cell::Cell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

thread_local! {
    static PINNED: Cell<Option<u64>> = const { Cell::new(None) };
}

/// The time now: the pinned second on a thread that pinned one, the wall clock elsewhere.
#[must_use]
pub fn now() -> SystemTime {
    PINNED.with(Cell::get).map_or_else(SystemTime::now, |secs| UNIX_EPOCH + Duration::from_secs(secs))
}

/// Seconds since the Unix epoch, by [`now`]; 0 for a clock set before 1970.
#[must_use]
pub fn unix_now() -> u64 {
    now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Holds this thread's clock at `secs` since the epoch until the pin is dropped.
#[must_use = "the clock is pinned only while the pin lives"]
pub fn pin(secs: u64) -> Pin {
    Pin { before: PINNED.with(|p| p.replace(Some(secs))) }
}

/// A pinned clock; dropping it gives the thread back the clock it had.
pub struct Pin {
    before: Option<u64>,
}

impl Drop for Pin {
    fn drop(&mut self) {
        PINNED.with(|p| p.set(self.before));
    }
}

#[cfg(test)]
mod tests;
