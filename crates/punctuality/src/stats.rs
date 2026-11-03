//! The aggregate itself: one histogram per cell, exact quantiles, bounded memory.
//!
//! A cell is a (station, train type, hour of day, weekend) bucket, and what it holds
//! is a count per delay minute rather than the delay values themselves. That choice is
//! what makes a single pass over ~120M stop records fit in memory: delays are small
//! integers, so a fixed array of counters is a complete description of the
//! distribution, and quantiles read straight off the cumulative counts instead of
//! needing the samples kept around and sorted.
//!
//! Measured against the real data (2026-06, 14.75M rows) before the window was chosen:
//! delays span -120..1421 minutes, but 99.906% of them land in [-5, 120]. So the two
//! open-ended edge buckets absorb roughly one row in a thousand, and every quantile
//! anyone asks for in practice (p50, p90) sits far inside the closed range. The mean is
//! kept as a running sum and is exact regardless of bucketing.

/// `< -5` (very early), then one bucket per minute from -5..=120, then `> 120`.
pub const BUCKETS: usize = 128;
const MIN_MINUTE: i32 = -5;
const MAX_MINUTE: i32 = 120;

fn bucket_of(delay: i32) -> usize {
    if delay < MIN_MINUTE {
        0
    } else if delay > MAX_MINUTE {
        BUCKETS - 1
    } else {
        (delay - MIN_MINUTE) as usize + 1
    }
}

/// The delay a bucket stands for. The two edge buckets are open-ended, so they report
/// their boundary — a quantile landing there is a floor, not a point estimate, which
/// `Cell::quantile_is_saturated` exists to make visible rather than quietly plausible.
fn minute_of(bucket: usize) -> i32 {
    if bucket == 0 {
        MIN_MINUTE - 1
    } else if bucket == BUCKETS - 1 {
        MAX_MINUTE + 1
    } else {
        MIN_MINUTE + (bucket as i32 - 1)
    }
}

/// One station/type/hour/weekend bucket's delay distribution.
#[derive(Clone)]
pub struct Cell {
    counts: [u32; BUCKETS],
    /// Stops with a delay reading. Cancellations are counted separately and are NOT in
    /// here: a cancelled train has no delay, and folding it in as "0 minutes late"
    /// would make the worst outcome improve the statistic.
    pub n: u64,
    pub canceled: u64,
    sum: i64,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            counts: [0; BUCKETS],
            n: 0,
            canceled: 0,
            sum: 0,
        }
    }
}

impl Cell {
    pub fn record(&mut self, delay: i32, canceled: bool) {
        if canceled {
            self.canceled += 1;
            return;
        }
        self.counts[bucket_of(delay)] += 1;
        self.n += 1;
        self.sum += delay as i64;
    }

    /// Reconstructs a cell from a persisted histogram and exact running sum.
    ///
    /// Incremental ingest needs to merge a new month into the materialized aggregate
    /// without reading every old parquet file again. Reject malformed persisted data
    /// rather than turning a bad row into a plausible statistic.
    pub fn from_parts(counts: &[i32], n: u64, canceled: u64, sum: i64) -> Option<Self> {
        if counts.len() != BUCKETS || counts.iter().any(|count| *count < 0) {
            return None;
        }
        let mut restored = [0u32; BUCKETS];
        for (slot, count) in restored.iter_mut().zip(counts) {
            *slot = u32::try_from(*count).ok()?;
        }
        let counted: u64 = restored.iter().map(|count| *count as u64).sum();
        (counted == n).then_some(Self {
            counts: restored,
            n,
            canceled,
            sum,
        })
    }

    /// The exact delay sum persisted beside the public mean so incremental merges do
    /// not repeatedly round a mean back into a sum.
    pub fn sum_delay(&self) -> i64 {
        self.sum
    }

    /// Folds another cell in. Histograms add, which is the property that lets ingest
    /// run file by file and still produce one exact distribution over the whole window.
    pub fn merge(&mut self, other: &Cell) {
        for (a, b) in self.counts.iter_mut().zip(other.counts.iter()) {
            *a += *b;
        }
        self.n += other.n;
        self.canceled += other.canceled;
        self.sum += other.sum;
    }

    pub fn mean(&self) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        self.sum as f64 / self.n as f64
    }

    /// Lower quantile: the smallest delay whose cumulative share reaches `q`.
    pub fn quantile(&self, q: f64) -> i32 {
        if self.n == 0 {
            return 0;
        }
        let target = (q * self.n as f64).ceil() as u64;
        let mut seen = 0u64;
        for (i, c) in self.counts.iter().enumerate() {
            seen += *c as u64;
            if seen >= target {
                return minute_of(i);
            }
        }
        minute_of(BUCKETS - 1)
    }

    /// True when this quantile fell into an open-ended edge bucket, so the number is a
    /// bound rather than a value. Callers that print a quantile should say so.
    pub fn quantile_is_saturated(&self, q: f64) -> bool {
        let m = self.quantile(q);
        !(MIN_MINUTE..=MAX_MINUTE).contains(&m)
    }

    /// Share of non-cancelled stops at least `minutes` late. Six is the interesting
    /// threshold because it is the one DB reports itself against: a train under six
    /// minutes counts as punctual in their own statistics.
    /// The histogram as it goes to Postgres. `i32` because that is the widest
    /// integer an `INTEGER[]` column holds, and a bucket count cannot exceed it
    /// for any station DB has ever published.
    pub fn counts_i32(&self) -> Vec<i32> {
        self.counts.iter().map(|c| *c as i32).collect()
    }

    /// The same exceedance `share_at_least` computes, from a stored bucket array
    /// rather than a live `Cell`. Separate function because the reader has a
    /// `Vec<i32>` out of Postgres and no `Cell` to put it back into, and going
    /// through a reconstructed `Cell` would invite the two to disagree.
    ///
    /// Returns `None` when the array is absent or the wrong length: a row written
    /// before the array was persisted must read as "cannot answer", never as zero.
    pub fn share_at_least_from_counts(counts: &[i32], minutes: i32) -> Option<f64> {
        if counts.len() != BUCKETS {
            return None;
        }
        let total: i64 = counts.iter().map(|c| *c as i64).sum();
        if total == 0 {
            return Some(0.0);
        }
        let late: i64 = (0..BUCKETS)
            .filter(|b| minute_of(*b) >= minutes)
            .map(|b| counts[b] as i64)
            .sum();
        Some(late as f64 / total as f64)
    }

    pub fn share_at_least(&self, minutes: i32) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        let from = bucket_of(minutes);
        let late: u64 = self.counts[from..].iter().map(|c| *c as u64).sum();
        late as f64 / self.n as f64
    }

    /// Share of scheduled stops that were cancelled — over stops *including*
    /// cancellations, unlike every other statistic here, because that is the question
    /// being asked ("how often does this not run at all").
    pub fn cancel_rate(&self) -> f64 {
        let total = self.n + self.canceled;
        if total == 0 {
            return 0.0;
        }
        self.canceled as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_of(delays: &[i32]) -> Cell {
        let mut c = Cell::default();
        for d in delays {
            c.record(*d, false);
        }
        c
    }

    #[test]
    fn quantiles_are_exact_inside_the_window() {
        let c = cell_of(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(c.quantile(0.5), 4);
        assert_eq!(c.quantile(0.9), 8);
        assert_eq!(c.quantile(1.0), 9);
        assert!(!c.quantile_is_saturated(0.9));
    }

    #[test]
    fn merging_histograms_equals_aggregating_once() {
        let all = cell_of(&[1, 5, 5, 20, 60, 90, 2, 2]);
        let mut split = cell_of(&[1, 5, 5, 20]);
        split.merge(&cell_of(&[60, 90, 2, 2]));
        assert_eq!(split.n, all.n);
        assert_eq!(split.quantile(0.5), all.quantile(0.5));
        assert_eq!(split.quantile(0.9), all.quantile(0.9));
        assert!((split.mean() - all.mean()).abs() < f64::EPSILON);
    }

    #[test]
    fn a_cancellation_is_not_a_punctual_train() {
        let mut c = Cell::default();
        c.record(0, false);
        c.record(0, true);
        // The cancelled stop must not enter n, the mean, or the late share -- otherwise
        // the worst possible outcome would improve every one of them.
        assert_eq!(c.n, 1);
        assert_eq!(c.canceled, 1);
        assert_eq!(c.mean(), 0.0);
        assert_eq!(c.share_at_least(6), 0.0);
        assert!((c.cancel_rate() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn the_edge_buckets_report_themselves_as_bounds() {
        let c = cell_of(&[500, 600, 700]);
        assert!(c.quantile_is_saturated(0.5));
        assert_eq!(c.quantile(0.5), MAX_MINUTE + 1);

        let early = cell_of(&[-40, -50]);
        assert!(early.quantile_is_saturated(0.5));
        assert_eq!(early.quantile(0.5), MIN_MINUTE - 1);
    }

    #[test]
    fn the_mean_survives_values_the_histogram_clamps() {
        // 1421 is the real maximum observed in 2026-06. It clamps into the overflow
        // bucket for quantile purposes, but the running sum keeps the mean honest.
        let c = cell_of(&[0, 1421]);
        assert!((c.mean() - 710.5).abs() < f64::EPSILON);
    }

    #[test]
    /// The stored path must agree with the live one exactly, or the six-minute
    /// column and the histogram would be two sources for one number. This is the
    /// equality the whole persistence change rests on.
    fn stored_counts_answer_the_same_question_as_the_live_cell() {
        let cell = cell_of(&[-2, 0, 0, 1, 3, 5, 6, 6, 7, 12, 20, 45, 90]);
        let stored = cell.counts_i32();
        for minutes in [0, 1, 3, 5, 6, 7, 10, 15, 30, 60, 120] {
            assert_eq!(
                Cell::share_at_least_from_counts(&stored, minutes),
                Some(cell.share_at_least(minutes)),
                "threshold {minutes} disagrees between the stored array and the live cell"
            );
        }
    }

    #[test]
    /// A row written before the array was persisted must read as "cannot answer".
    /// Zero would be a claim that no train was ever that late.
    fn an_absent_or_malformed_histogram_is_not_zero_risk() {
        assert_eq!(Cell::share_at_least_from_counts(&[], 6), None);
        assert_eq!(Cell::share_at_least_from_counts(&[1, 2, 3], 6), None);
        // A present but empty histogram is a real zero: nothing was observed.
        assert_eq!(
            Cell::share_at_least_from_counts(&vec![0; BUCKETS], 6),
            Some(0.0)
        );
    }

    #[test]
    fn late_share_counts_from_six_minutes_inclusive() {
        let c = cell_of(&[5, 6, 7]);
        assert!((c.share_at_least(6) - 2.0 / 3.0).abs() < 1e-12);
    }
}
