#![feature(uint_gather_scatter_bits)]

macro_rules! make_base_mask {
    ($encoded:ty, $decoded:ty, $n:expr) => {
        const {
            let mut mask: $encoded = 0;
            let mut i = 0;

            while i < <$decoded>::BITS {
                mask |= 1 << ($n * i);
                i += 1;
            }

            mask
        }
    };
}

pub trait Morton {
    type Code;

    fn morton_encode(self) -> Self::Code;
    fn morton_decode(code: Self::Code) -> Self;
}

macro_rules! impl_morton {
    ($encoded:ty, $decoded:ty, [$($i:literal),*], $n:literal) => {
        const _: () = {
            const MASKS: [$encoded; $n] = [
                $(make_base_mask!($encoded, $decoded, $n) << ($n - $i - 1)),*
            ];

            impl Morton for [$decoded; $n] {
                type Code = $encoded;

                #[inline]
                fn morton_encode(self) -> $encoded {
                    $(
                        (self[$i] as $encoded).deposit_bits(MASKS[$i])
                    )|*
                }

                #[inline]
                fn morton_decode(code: $encoded) -> [$decoded; $n] {
                    [
                        $(
                            code.extract_bits(MASKS[$i]) as $decoded,
                        )*
                    ]
                }
            }
        };
    };
}

#[inline]
pub fn encode<T>(x: T) -> T::Code
where
    T: Morton,
{
    x.morton_encode()
}

#[inline]
pub fn decode<T>(x: T::Code) -> T
where
    T: Morton,
{
    T::morton_decode(x)
}

impl_morton!(u16, u8, [0, 1], 2);
impl_morton!(u32, u8, [0, 1, 2], 3);
impl_morton!(u32, u8, [0, 1, 2, 3], 4);

impl_morton!(u32, u16, [0, 1], 2);
impl_morton!(u64, u16, [0, 1, 2], 3);
impl_morton!(u64, u16, [0, 1, 2, 3], 4);

#[cfg(test)]
mod tests {
    use crate::Morton;

    // 123 = 0b001111011 -> 0b00010101010001010
    // 456 = 0b111001000 -> 0b10101000001000000
    // 0b000010101010001010 | 0b010101000001000000 = 96970
    const EXAMPLE_U16_2: ([u16; 2], u32) = ([123, 456], 96970);

    // 123 = 0b0001111011 -> 0b0000000100100100100000100100
    // 456 = 0b0111001000 -> 0b0010010010000000010000000000
    // 789 = 0b1100010101 -> 0b1001000000000001000001000001
    // 0b0000000100100100100000100100 | 0b0010010010000000010000000000 |
    // 0b1001000000000001000001000001 = 190471269
    const EXAMPLE_U16_3: ([u16; 3], u64) = ([123, 456, 789], 190471269);

    #[test]
    fn test_examples_against_morton_encoding() {
        assert_eq!(
            morton_encoding::morton_encode(EXAMPLE_U16_2.0),
            EXAMPLE_U16_2.1
        );
        assert_eq!(
            morton_encoding::morton_decode::<u16, 2>(EXAMPLE_U16_2.1),
            EXAMPLE_U16_2.0
        );

        assert_eq!(
            morton_encoding::morton_encode(EXAMPLE_U16_3.0),
            EXAMPLE_U16_3.1
        );
        assert_eq!(
            morton_encoding::morton_decode::<u16, 3>(EXAMPLE_U16_3.1),
            EXAMPLE_U16_3.0
        );
    }

    #[test]
    fn test_encode_u16_2() {
        let x: [u16; 2] = [123, 456];

        assert_eq!(x.morton_encode(), 96970);
    }

    #[test]
    fn test_encode_u16_3() {
        let x: [u16; 3] = [123, 456, 789];

        assert_eq!(x.morton_encode(), 190471269);
    }

    #[test]
    fn test_decode_u16_2() {
        assert_eq!(<[u16; 2]>::morton_decode(96970), [123, 456]);
    }

    #[test]
    fn test_decode_u16_3() {
        assert_eq!(<[u16; 3]>::morton_decode(190471269), [123, 456, 789]);
    }
}
