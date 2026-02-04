pub mod bitmatrix_transpose;
pub mod image;
pub mod noise;
pub mod oneshot;
pub mod serde;
pub mod sparse_vec;
pub mod stats_alloc;
#[cfg(feature = "tokio")]
pub mod tokio;

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

pub fn format_size<T>(value: T) -> humansize::SizeFormatter<T, humansize::FormatSizeOptions>
where
    T: humansize::ToF64 + humansize::Unsigned,
{
    humansize::SizeFormatter::new(value, humansize::BINARY)
}

#[macro_export]
macro_rules! define_atomic_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(std::num::NonZero<usize>);

        impl $name {
            fn new() -> Self {
                static NEXT: std::sync::atomic::AtomicUsize =
                    std::sync::atomic::AtomicUsize::new(1);

                let next = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let id = std::num::NonZero::<usize>::new(next)
                    .unwrap_or_else(|| panic!("ID overflow for `{}`", stringify!($name)));

                Self(id)
            }
        }
    };
}

#[inline]
pub fn bitmask(n: usize) -> u64 {
    assert!(n <= 64);
    if n < 64 {
        (1 << n) - 1
    }
    else if n == 64 {
        u64::MAX
    }
    else {
        unreachable!();
    }
}
