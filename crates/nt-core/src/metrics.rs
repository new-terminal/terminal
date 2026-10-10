//! Speed metrics from the window, summarized for `app.log`.

use std::time::Duration;

/// Keypress samples are summarized once per this many keys.
const KEYPRESS_BATCH: usize = 200;

/// Nearest-rank percentiles of one batch of keypress-to-frame samples, in
/// whole milliseconds rounded up, so a rounded value never hides a missed
/// budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeypressSummary {
    pub p50_ms: u128,
    pub p95_ms: u128,
    pub count: usize,
}

#[derive(Debug, Default)]
pub struct KeypressSamples {
    samples: Vec<Duration>,
}

impl KeypressSamples {
    /// Adds one sample. Returns the batch summary, and starts a new batch,
    /// when the batch is full.
    pub fn push(&mut self, sample: Duration) -> Option<KeypressSummary> {
        self.samples.push(sample);
        (self.samples.len() >= KEYPRESS_BATCH).then(|| self.take())
    }

    /// Summarizes and clears a partial batch. `None` when it is empty.
    pub fn flush(&mut self) -> Option<KeypressSummary> {
        (!self.samples.is_empty()).then(|| self.take())
    }

    fn take(&mut self) -> KeypressSummary {
        let summary = summarize(&self.samples);
        self.samples.clear();
        summary
    }
}

fn summarize(samples: &[Duration]) -> KeypressSummary {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    KeypressSummary {
        p50_ms: whole_ms_rounded_up(nearest_rank(&sorted, 50)),
        p95_ms: whole_ms_rounded_up(nearest_rank(&sorted, 95)),
        count: sorted.len(),
    }
}

/// The nearest-rank percentile of a sorted, non-empty slice.
fn nearest_rank(sorted: &[Duration], percent: usize) -> Duration {
    let rank = (percent * sorted.len()).div_ceil(100).max(1);
    sorted[rank - 1]
}

pub const fn whole_ms_rounded_up(duration: Duration) -> u128 {
    duration.as_micros().div_ceil(1000)
}
