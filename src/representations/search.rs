use std::cmp::Ordering;

pub enum SearchStatus {
    Bounds(usize, usize),
    Found(usize),
}

use Ordering::*;
use SearchStatus::*;

/// Strategy pattern for customizing binary search behavior.
///
/// This trait allows different implementations to vary how binary search progresses and terminates,
/// enabling variants like exact-match search, lower bound, and upper bound with a single core algorithm.
///
/// Implementations customize behavior through two key methods:
/// - `step()`: Determines the search direction at each iteration
/// - `on_convergence()`: Handles termination when the search window collapses
pub trait SearchStrategy<T: Ord> {
    /// Determines how to adjust search bounds on each iteration.
    ///
    /// # Arguments
    /// - `idx`: Current midpoint being examined
    /// - `low`, `high`: Current search window boundaries
    /// - `cmp`: Comparison result between the element at `idx` and the target
    ///
    /// # Returns
    /// - `SearchStatus::Bounds(l, h)` to continue searching in range `[l, h)`
    /// - `SearchStatus::Found(x)` to terminate and return the index
    fn step(&self, idx: usize, low: usize, high: usize, cmp: std::cmp::Ordering) -> SearchStatus;

    /// Determines the final result when the search window has collapsed (`low == high`).
    ///
    /// # Arguments
    /// - `idx`: The final position where convergence occurred
    /// - `cmp`: Comparison result at convergence
    ///
    /// # Returns
    /// `Some(idx)` with the final position or `None` if no valid result exists
    fn on_convergence(&self, idx: usize, cmp: std::cmp::Ordering) -> Option<usize>;

    /// Executes binary search with a customizable strategy.
    ///
    /// This method implements the core binary search loop, repeatedly calling `step()` to adjust
    /// bounds and `on_convergence()` to determine the result when bounds collapse.
    ///
    /// # Arguments
    /// - `get_key`: Closure that retrieves the key at a given index
    /// - `key`: The target value to search for
    /// - `start`, `end`: Initial search range (exclusive upper bound)
    ///
    /// # Returns
    /// `Some(idx)` if a valid position is found, `None` otherwise
    ///
    /// # Example
    /// ```ignore
    /// let vec = [2, 5, 5, 8];
    /// let strategy = BinarySearch::default();
    /// let result = strategy.search(|i| vec[i], 5, 0, 3);
    /// ```
    fn search<F>(&self, get_key: F, key: T, start: usize, end: usize) -> Option<usize>
    where
        F: Fn(usize) -> T,
    {
        let mut low = start;
        let mut high = end;
        loop {
            let idx = (high - low) / 2 + low;
            let this_key = get_key(idx);

            let cmp = this_key.cmp(&key);

            if low == high {
                return self.on_convergence(idx, cmp);
            }

            match self.step(idx, low, high, this_key.cmp(&key)) {
                Bounds(l, h) => {
                    low = l;
                    high = h;
                }
                Found(x) => return Some(x),
            }
        }
    }
}

/// Implements exact-match binary search.
///
/// Returns `Some(idx)` only if the key is found at the exact position; otherwise returns `None`.
/// This is the standard binary search algorithm used when you need to determine if a value exists
/// in a sorted sequence.
///
/// # Example
///
/// ```ignore
/// let vec = [2, 5, 5, 8];
/// let strategy = BinarySearch::default();
///
/// assert_eq!(Some(0), strategy.search(|i| vec[i], 2, 0, 3));
/// assert_eq!(Some(1), strategy.search(|i| vec[i], 5, 0, 3));
/// assert_eq!(Some(3), strategy.search(|i| vec[i], 8, 0, 3));
/// assert_eq!(None, strategy.search(|i| vec[i], 6, 0, 3));
/// ```
#[derive(Debug)]
pub struct BinarySearch;

impl Default for BinarySearch {
    fn default() -> Self {
        BinarySearch {}
    }
}

impl<T: Ord> SearchStrategy<T> for BinarySearch {
    fn step(
        &self,
        idx: usize,
        low: usize,
        high: usize,
        compare: std::cmp::Ordering,
    ) -> SearchStatus {
        match compare {
            Equal => return Found(idx),
            Less => return Bounds(idx + 1, high),
            Greater => {
                if idx == low {
                    Bounds(low, high - 1)
                } else {
                    Bounds(low, idx - 1)
                }
            }
        }
    }

    fn on_convergence(&self, idx: usize, cmp: std::cmp::Ordering) -> Option<usize> {
        match cmp {
            Equal => Some(idx),
            _ => None,
        }
    }
}

/// Finds the lower bound (leftmost insertion point) for a key in a sorted sequence.
///
/// Returns the index of the leftmost position where the key could be inserted to maintain sorted order.
/// If the key exists multiple times, returns the index of the first occurrence. Useful for
/// partitioning operations or finding the start of a range of equal elements.
///
/// # Example
///
/// ```ignore
/// let vec = [2, 5, 5, 8];
/// let strategy = LowerPartitionSearch::default();
///
/// assert_eq!(Some(0), strategy.search(|i| vec[i], 2, 0, 3));   // leftmost 2
/// assert_eq!(Some(1), strategy.search(|i| vec[i], 5, 0, 3));   // leftmost 5
/// assert_eq!(Some(3), strategy.search(|i| vec[i], 8, 0, 3));   // leftmost 8
/// assert_eq!(Some(3), strategy.search(|i| vec[i], 6, 0, 3));   // where 6 would go
/// ```
#[derive(Debug)]
pub struct LowerPartitionSearch;

impl Default for LowerPartitionSearch {
    fn default() -> Self {
        LowerPartitionSearch {}
    }
}

impl<T: Ord> SearchStrategy<T> for LowerPartitionSearch {
    fn step(&self, idx: usize, low: usize, high: usize, cmp: std::cmp::Ordering) -> SearchStatus {
        match cmp {
            Equal => Bounds(low, idx),
            Less => Bounds(idx + 1, high),
            Greater => {
                if idx == low {
                    Bounds(low, high - 1)
                } else {
                    Bounds(low, idx - 1)
                }
            }
        }
    }

    fn on_convergence(&self, idx: usize, cmp: std::cmp::Ordering) -> Option<usize> {
        match cmp {
            Less => Some(idx + 1),
            _ => Some(idx),
        }
    }
}

/// Finds the upper bound (rightmost insertion point) for a key in a sorted sequence.
///
/// Returns the index of the rightmost position where the key could be inserted to maintain sorted order.
/// If the key exists multiple times, returns the index just after the last occurrence. Useful for
/// partitioning operations or finding the end of a range of equal elements.
///
/// # Example
///
/// ```ignore
/// let vec = [2, 5, 5, 8];
/// let strategy = UpperPartitionSearch::default();
///
/// assert_eq!(Some(1), strategy.search(|i| vec[i], 2, 0, 3));   // after 2
/// assert_eq!(Some(3), strategy.search(|i| vec[i], 5, 0, 3));   // after last 5
/// assert_eq!(Some(4), strategy.search(|i| vec[i], 8, 0, 3));   // after 8
/// assert_eq!(Some(3), strategy.search(|i| vec[i], 6, 0, 3));   // where 6 would go
/// ```
#[derive(Debug)]
pub struct UpperPartitionSearch;

impl Default for UpperPartitionSearch {
    fn default() -> Self {
        UpperPartitionSearch {}
    }
}

impl<T: Ord> SearchStrategy<T> for UpperPartitionSearch {
    fn on_convergence(&self, idx: usize, cmp: std::cmp::Ordering) -> Option<usize> {
        match cmp {
            Greater => Some(idx),
            _ => Some(idx + 1),
        }
    }

    fn step(&self, idx: usize, low: usize, high: usize, cmp: std::cmp::Ordering) -> SearchStatus {
        match cmp {
            Equal => Bounds(idx + 1, high),
            Less => Bounds(idx + 1, high),
            Greater => {
                if idx == low {
                    Bounds(low, high - 1)
                } else {
                    Bounds(low, idx - 1)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary() {
        let vec = vec![2, 5, 5, 5, 7];

        let strat = BinarySearch::default();

        assert_eq!(None, strat.search(|i| *vec.get(i).unwrap(), 0, 0, 4));
        assert_eq!(Some(0), strat.search(|i| *vec.get(i).unwrap(), 2, 0, 4))
    }

    #[test]
    fn test_lower_partition() {
        let vec = vec![2, 5, 5, 5, 7];
        let strat = LowerPartitionSearch::default();

        assert_eq!(Some(0), strat.search(|i| *vec.get(i).unwrap(), 0, 0, 4));
        assert_eq!(Some(0), strat.search(|i| *vec.get(i).unwrap(), 2, 0, 4));
        assert_eq!(Some(1), strat.search(|i| *vec.get(i).unwrap(), 3, 0, 4));
        assert_eq!(Some(1), strat.search(|i| *vec.get(i).unwrap(), 5, 0, 4));
        assert_eq!(Some(4), strat.search(|i| *vec.get(i).unwrap(), 6, 0, 4));
        assert_eq!(Some(4), strat.search(|i| *vec.get(i).unwrap(), 7, 0, 4));
        assert_eq!(Some(5), strat.search(|i| *vec.get(i).unwrap(), 9, 0, 4));
    }

    #[test]
    fn test_upper_partition() {
        let vec: Vec<i32> = vec![2, 5, 5, 5, 7];
        let strat = UpperPartitionSearch::default();

        assert_eq!(Some(0), strat.search(|i| *vec.get(i).unwrap(), 0, 0, 4));
        assert_eq!(Some(1), strat.search(|i| *vec.get(i).unwrap(), 2, 0, 4));
        assert_eq!(Some(1), strat.search(|i| *vec.get(i).unwrap(), 3, 0, 4));
        assert_eq!(Some(4), strat.search(|i| *vec.get(i).unwrap(), 5, 0, 4));
        assert_eq!(Some(4), strat.search(|i| *vec.get(i).unwrap(), 6, 0, 4));
        assert_eq!(Some(5), strat.search(|i| *vec.get(i).unwrap(), 7, 0, 4));
        assert_eq!(Some(5), strat.search(|i| *vec.get(i).unwrap(), 9, 0, 4));
    }
}
