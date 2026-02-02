#![feature(uint_gather_scatter_bits)]

const MASKS2: [u32; 2] = [0xaa_aa_aa_aa, 0x55_55_55_55];
const MASKS3: [u64; 3] = [0x924_924_924_924, 0x492_492_492_492, 0x249_249_249_249];

#[inline]
pub const fn encode2(x: [u16; 2]) -> u32 {
    (x[0] as u32).deposit_bits(MASKS2[0]) | (x[1] as u32).deposit_bits(MASKS2[1])
}

#[inline]
pub const fn encode3(x: [u16; 3]) -> u64 {
    (x[0] as u64).deposit_bits(MASKS3[0])
        | (x[1] as u64).deposit_bits(MASKS3[1])
        | (x[2] as u64).deposit_bits(MASKS3[2])
}

#[cfg(test)]
mod tests {
    use crate::{
        encode2,
        encode3,
    };

    #[test]
    fn test_encode2() {
        let x: [u16; 2] = [123, 456];

        // 123 = 0b001111011 -> 0b00010101010001010
        // 456 = 0b111001000 -> 0b10101000001000000
        // 0b000010101010001010 | 0b010101000001000000 = 96970

        assert_eq!(encode2(x), 96970);
    }

    #[test]
    fn test_encode3() {
        let x: [u16; 3] = [123, 456, 789];

        // 123 = 0b0001111011 -> 0b0000000100100100100000100100
        // 456 = 0b0111001000 -> 0b0010010010000000010000000000
        // 789 = 0b1100010101 -> 0b1001000000000001000001000001
        // 0b0000000100100100100000100100 | 0b0010010010000000010000000000 |
        // 0b1001000000000001000001000001 = 190471269

        dbg!(morton_encoding::morton_encode(x));
        assert_eq!(encode3(x), 190471269);
    }
}
