//! ISAAC-64 stream used by SNS media. The generator is independently expressed
//! from Bob Jenkins' public-domain ISAAC-64 specification (burtleburtle.net).
//! All arithmetic is modulo 2^64; SNS consumes each result in big-endian order.
use zeroize::Zeroize;
pub struct Isaac64 {
    memory: [u64; 256],
    results: [u64; 256],
    a: u64,
    b: u64,
    c: u64,
    left: usize,
}
fn mix(x: &mut [u64; 8]) {
    x[0] = x[0].wrapping_sub(x[4]);
    x[5] ^= x[7] >> 9;
    x[7] = x[7].wrapping_add(x[0]);
    x[1] = x[1].wrapping_sub(x[5]);
    x[6] ^= x[0] << 9;
    x[0] = x[0].wrapping_add(x[1]);
    x[2] = x[2].wrapping_sub(x[6]);
    x[7] ^= x[1] >> 23;
    x[1] = x[1].wrapping_add(x[2]);
    x[3] = x[3].wrapping_sub(x[7]);
    x[0] ^= x[2] << 15;
    x[2] = x[2].wrapping_add(x[3]);
    x[4] = x[4].wrapping_sub(x[0]);
    x[1] ^= x[3] >> 14;
    x[3] = x[3].wrapping_add(x[4]);
    x[5] = x[5].wrapping_sub(x[1]);
    x[2] ^= x[4] << 20;
    x[4] = x[4].wrapping_add(x[5]);
    x[6] = x[6].wrapping_sub(x[2]);
    x[3] ^= x[5] >> 17;
    x[5] = x[5].wrapping_add(x[6]);
    x[7] = x[7].wrapping_sub(x[3]);
    x[4] ^= x[6] << 14;
    x[6] = x[6].wrapping_add(x[7]);
}
impl Isaac64 {
    pub fn new(seed: u64) -> Self {
        Self::seeded(&[seed])
    }
    fn seeded(words: &[u64]) -> Self {
        let mut s = Self {
            memory: [0; 256],
            results: [0; 256],
            a: 0,
            b: 0,
            c: 0,
            left: 0,
        };
        s.results[..words.len()].copy_from_slice(words);
        let mut x = [0x9e3779b97f4a7c13u64; 8];
        for _ in 0..4 {
            mix(&mut x);
        }
        for pass in 0..2 {
            for i in (0..256).step_by(8) {
                for (j, word) in x.iter_mut().enumerate() {
                    *word = word.wrapping_add(if pass == 0 {
                        s.results[i + j]
                    } else {
                        s.memory[i + j]
                    });
                }
                mix(&mut x);
                s.memory[i..i + 8].copy_from_slice(&x);
            }
        }
        x.zeroize();
        s.refill();
        s
    }
    fn refill(&mut self) {
        self.c = self.c.wrapping_add(1);
        self.b = self.b.wrapping_add(self.c);
        for i in 0..256 {
            let x = self.memory[i];
            self.a = match i % 4 {
                0 => !(self.a ^ (self.a << 21)),
                1 => self.a ^ (self.a >> 5),
                2 => self.a ^ (self.a << 12),
                _ => self.a ^ (self.a >> 33),
            };
            self.a = self.a.wrapping_add(self.memory[(i + 128) % 256]);
            let y = self.memory[((x >> 3) & 255) as usize]
                .wrapping_add(self.a)
                .wrapping_add(self.b);
            self.memory[i] = y;
            self.b = self.memory[((y >> 11) & 255) as usize].wrapping_add(x);
            self.results[i] = self.b;
        }
        self.left = 256;
    }
    pub fn next_word(&mut self) -> u64 {
        if self.left == 0 {
            self.refill();
        }
        self.left -= 1;
        self.results[self.left]
    }
    pub fn xor(&mut self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(8) {
            let mut key = self.next_word().to_be_bytes();
            for (a, b) in chunk.iter_mut().zip(&key) {
                *a ^= *b;
            }
            key.zeroize();
        }
    }
}
impl Drop for Isaac64 {
    fn drop(&mut self) {
        self.memory.zeroize();
        self.results.zeroize();
        self.a.zeroize();
        self.b.zeroize();
        self.c.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_words_and_sns_byte_order() {
        // rand_isaac 0.4.0, test_isaac64_true_values_64, compared with Jenkins' reference.
        // https://docs.rs/rand_isaac/0.4.0/src/rand_isaac/isaac64.rs.html
        let expected = [
            15071495833797886820u64,
            7720185633435529318,
            10836773366498097981,
            5414053799617603544,
            12890513357046278984,
            17001051845652595546,
            9240803642279356310,
            12558996012687158051,
            14673053937227185542,
            1677046725350116783,
        ];
        let mut rng = Isaac64::seeded(&[1, 23, 456, 7890]);
        for word in expected {
            assert_eq!(rng.next_word(), word);
        }
        let mut bytes = [0u8; 80];
        Isaac64::seeded(&[1, 23, 456, 7890]).xor(&mut bytes);
        for (part, word) in bytes.chunks_exact(8).zip(expected) {
            assert_eq!(part, word.to_be_bytes());
        }
    }
}
