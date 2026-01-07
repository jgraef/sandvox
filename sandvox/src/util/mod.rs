pub mod image;
pub mod oneshot;

use std::ops::{
    Bound,
    Range,
    RangeBounds,
};

pub fn normalize_index_bounds(range: impl RangeBounds<usize>, len: usize) -> Range<usize> {
    let start = match range.start_bound() {
        Bound::Included(start) => *start,
        Bound::Excluded(start) => start + 1,
        Bound::Unbounded => 0,
    };

    let end = match range.end_bound() {
        Bound::Included(end) => end + 1,
        Bound::Excluded(end) => *end,
        Bound::Unbounded => len,
    };

    let end = end.max(start);

    Range { start, end }
}
