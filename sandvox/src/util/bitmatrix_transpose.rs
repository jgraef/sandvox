use crate::util::bitmask;

pub trait BitMatrix {
    fn len(&self) -> usize;
    fn get_mut_2(&mut self, rows: [usize; 2]) -> [&mut u64; 2];

    /// Taken from [`dsnet/matrix-transpose`][1]
    ///
    /// [1]: https://github.com/dsnet/matrix-transpose
    #[profiling::function]
    fn transpose(&mut self) {
        //let mut swap_width = 64;
        //let mut swap_mask = u64::MAX;

        let mut swap_width = self.len();
        if swap_width < 2 {
            return;
        }
        assert!(swap_width.is_power_of_two());

        let mut swap_mask = bitmask(swap_width);

        let mut outer_count = 1;

        while swap_width != 1 {
            swap_width >>= 1;
            swap_mask = swap_mask ^ (swap_mask >> swap_width);

            for outer in 0..outer_count {
                for inner in 0..swap_width {
                    let inner_offset = inner + outer * swap_width * 2;
                    let [x, y] = self.get_mut_2([inner_offset, inner_offset + swap_width]);

                    *x = ((*y << swap_width) & swap_mask) ^ *x;
                    *y = ((*x & swap_mask) >> swap_width) ^ *y;
                    *x = ((*y << swap_width) & swap_mask) ^ *x;
                }
            }

            outer_count <<= 1;
        }
    }
}

impl BitMatrix for [u64] {
    #[inline]
    fn len(&self) -> usize {
        <[_]>::len(self)
    }

    #[inline]
    fn get_mut_2(&mut self, rows: [usize; 2]) -> [&mut u64; 2] {
        slice_get_mut_2(self, rows)
    }
}

#[inline]
pub fn slice_get_mut_2<T>(slice: &mut [T], [i, j]: [usize; 2]) -> [&mut T; 2] {
    if i < j {
        let (left, right) = slice.split_at_mut(j);
        [&mut left[i], &mut right[0]]
    }
    else if j < i {
        let (left, right) = slice.split_at_mut(i);
        [&mut right[0], &mut left[j]]
    }
    else {
        panic!("Both indices can't be equal: {i} != {j}");
    }
}
