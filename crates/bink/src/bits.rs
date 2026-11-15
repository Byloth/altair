//! Lettore di bit: in Bink i bit si leggono dal meno significativo di ogni byte, e un valore
//! di più bit ha come bit basso il primo letto.

pub struct Bits<'a> {
    data: &'a [u8],
    /// Posizione in bit dall'inizio.
    pos: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Bits<'a> {
        Bits { data, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    /// Bit ancora da leggere (negativo se si è letto oltre la fine: i bit mancanti valgono zero).
    pub fn left(&self) -> isize {
        (self.data.len() * 8) as isize - self.pos as isize
    }

    /// I prossimi `n` bit (fino a 32) senza consumarli.
    #[inline]
    pub fn peek(&self, n: u32) -> u32 {
        let byte = self.pos >> 3;
        let mut window = 0u64;
        // Otto byte bastano per 32 bit più lo scarto dentro il primo byte.
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

    /// Avanza fino al prossimo multiplo di 32 bit.
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
        assert_eq!(b.get(5), 0); // oltre la fine: zeri
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
