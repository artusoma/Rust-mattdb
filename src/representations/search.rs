use std::cmp::Ordering;

pub enum SearchStatus {
    Bounds(usize, usize),
    Found(usize),
    NotFound,
}

use Ordering::*;
use SearchStatus::*;

/// What is difference between making T a generic and a type?
pub trait SearchStrategy<T: Ord> {
    fn step(&self, idx: usize, low: usize, high: usize, cmp: std::cmp::Ordering) -> SearchStatus;

    fn on_convergence(&self, idx: usize, cmp: std::cmp::Ordering) -> Option<usize>;

    fn search<'a, F>(&self, get_key: F, key: T, start: usize, end: usize) -> Option<usize>
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
                NotFound => return None,
                Found(x) => return Some(x),
            }
        }
    }
}

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

        assert_eq!(None, strat.search(|i| vec.get(i).unwrap(), &0, 0, 4));
        assert_eq!(Some(0), strat.search(|i| vec.get(i).unwrap(), &2, 0, 4))
    }

    #[test]
    fn test_lower_partition() {
        let vec = vec![2, 5, 5, 5, 7];
        let strat = LowerPartitionSearch::default();

        assert_eq!(Some(0), strat.search(|i| vec.get(i).unwrap(), &0, 0, 4));
        assert_eq!(Some(0), strat.search(|i| vec.get(i).unwrap(), &2, 0, 4));
        assert_eq!(Some(1), strat.search(|i| vec.get(i).unwrap(), &3, 0, 4));
        assert_eq!(Some(1), strat.search(|i| vec.get(i).unwrap(), &5, 0, 4));
        assert_eq!(Some(4), strat.search(|i| vec.get(i).unwrap(), &6, 0, 4));
        assert_eq!(Some(4), strat.search(|i| vec.get(i).unwrap(), &7, 0, 4));
        assert_eq!(Some(5), strat.search(|i| vec.get(i).unwrap(), &9, 0, 4));
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
