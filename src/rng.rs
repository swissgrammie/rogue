//! Tiny deterministic PRNG (xorshift64*), so maps are reproducible from a seed
//! without pulling in a dependency.

pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // A zero state is a fixed point for xorshift; nudge it away.
        Rng {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n`. Panics if `n == 0`.
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "below(0) has no result");
        (self.next_u64() % n as u64) as usize
    }

    /// Uniform in `lo..=hi`. Panics if `hi < lo`.
    pub fn range(&mut self, lo: usize, hi: usize) -> usize {
        assert!(hi >= lo, "empty range {lo}..={hi}");
        lo + self.below(hi - lo + 1)
    }

    /// True with probability `numerator / denominator`.
    pub fn chance(&mut self, numerator: u32, denominator: u32) -> bool {
        self.below(denominator as usize) < numerator as usize
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Rng;

    #[test]
    fn same_seed_same_sequence() {
        let a: Vec<usize> = (0..32).map(|_| Rng::new(7).below(100)).collect();
        let mut r = Rng::new(7);
        let b: Vec<usize> = (0..32).map(|_| r.below(100)).collect();
        assert_eq!(a[0], b[0]);

        let mut x = Rng::new(1234);
        let mut y = Rng::new(1234);
        for _ in 0..1000 {
            assert_eq!(x.below(1000), y.below(1000));
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut x = Rng::new(1);
        let mut y = Rng::new(2);
        let xs: Vec<usize> = (0..16).map(|_| x.below(1000)).collect();
        let ys: Vec<usize> = (0..16).map(|_| y.below(1000)).collect();
        assert_ne!(xs, ys);
    }

    #[test]
    fn range_is_inclusive_and_in_bounds() {
        let mut r = Rng::new(99);
        for _ in 0..1000 {
            let v = r.range(3, 7);
            assert!((3..=7).contains(&v), "{v} out of range");
        }
        assert_eq!(r.range(5, 5), 5);
    }
}
