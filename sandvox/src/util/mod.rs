pub mod image;
pub mod noise;
pub mod oneshot;
pub mod serde;

use std::ops::{
    Add,
    Bound,
    Mul,
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

pub fn lerp<T>(x0: T, x1: T, t: f32) -> T
where
    T: Mul<f32, Output = T> + Add<T, Output = T>,
{
    x0 * (1.0 - t) + x1 * t
}
