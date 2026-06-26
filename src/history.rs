//! A fixed-capacity ring buffer for time-series data feeding the graphs.

use std::collections::VecDeque;

/// A bounded history of `f64` samples (oldest at front, newest at back).
#[derive(Clone, Debug)]
pub struct History {
    data: VecDeque<f64>,
    capacity: usize,
}

impl Default for History {
    fn default() -> Self {
        History::new(128)
    }
}

impl History {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            data: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Append a sample, evicting the oldest if at capacity.
    pub fn push(&mut self, value: f64) {
        if self.data.len() == self.capacity {
            self.data.pop_front();
        }
        self.data.push_back(value);
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// The most recent sample, or `0.0` if empty.
    pub fn last(&self) -> f64 {
        self.data.back().copied().unwrap_or(0.0)
    }

    /// The largest sample currently held, or `0.0` if empty.
    pub fn max(&self) -> f64 {
        self.data.iter().copied().fold(0.0_f64, f64::max)
    }

    /// Return the most recent `n` samples as a contiguous vector, oldest first.
    /// If fewer than `n` exist, returns all of them.
    pub fn tail(&self, n: usize) -> Vec<f64> {
        let len = self.data.len();
        let start = len.saturating_sub(n);
        self.data.iter().skip(start).copied().collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = &f64> {
        self.data.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_oldest() {
        let mut h = History::new(3);
        for v in [1.0, 2.0, 3.0, 4.0] {
            h.push(v);
        }
        assert_eq!(h.len(), 3);
        assert_eq!(h.tail(3), vec![2.0, 3.0, 4.0]);
        assert_eq!(h.last(), 4.0);
        assert_eq!(h.max(), 4.0);
    }

    #[test]
    fn tail_underflow() {
        let mut h = History::new(10);
        h.push(5.0);
        assert_eq!(h.tail(4), vec![5.0]);
        assert_eq!(h.max(), 5.0);
    }

    #[test]
    fn empty_defaults() {
        let h = History::new(4);
        assert!(h.is_empty());
        assert_eq!(h.last(), 0.0);
        assert_eq!(h.max(), 0.0);
        assert!(h.tail(3).is_empty());
    }
}
