//! Bit reader: in Bink, bits are read starting from the least significant of each byte, and a
//! multi-bit value has the first bit read as its low bit.

pub struct Bits<'a> {
    data: &'a [u8],
    /// Position in bits from the start.
    pos: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Bits<'a> {
        Bits { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    /// Bits still to be read (negative if we have read past the end: the missing bits are zero).
    pub fn left(&self) -> isize {
        (self.data.len() * 8) as isize - self.pos as isize
    }

    /// The next `n` bits (up to 32) without consuming them.
    #[inline]
    pub fn peek(&self, n: u32) -> u32 {
        let byte = self.pos >> 3;
        let mut window = 0u64;
        // Eight bytes are enough for 32 bits plus the offset inside the first byte.
        if byte + 8 <= self.data.len() {
            window = u64::from_le_bytes(self.data[byte..byte + 8].try_into().unwrap());
        } else {
            for (i, b) in self.data.iter().skip(byte).take(8).enumerate() {
                window |= (*b as u64) << (8 * i);
            }
        }
        ((window >> (self.pos & 7)) & ((1u64 << n) - 1)) as u32
    }

    #[inline]
    pub fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    #[inline]
    pub fn get(&mut self, n: u32) -> u32 {
        let v = self.peek(n);
        self.pos += n as usize;
        v
    }

    #[inline]
    pub fn bit(&mut self) -> bool {
        self.get(1) != 0
    }

    /// Advances to the next multiple of 32 bits.
    pub fn align32(&mut self) {
        self.pos = (self.pos + 31) & !31;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_from_the_low_bit() {
        let mut b = Bits::new(&[0b1010_0101, 0xff, 0x00, 0x80]);
        assert_eq!(b.get(1), 1);
        assert_eq!(b.get(3), 0b010);
        assert_eq!(b.get(8), 0b1111_1010);
        assert_eq!(b.get(4), 0b1111);
        assert_eq!(b.get(16), 0x8000);
        assert_eq!(b.left(), 0);
        assert_eq!(b.get(5), 0); // past the end: zeros
    }

    #[test]
    fn aligns_to_32_bits() {
        let data = [0u8; 12];
        let mut b = Bits::new(&data);
        b.align32();
        assert_eq!(b.position(), 0);
        b.get(1);
        b.align32();
        assert_eq!(b.position(), 32);
    }
}
