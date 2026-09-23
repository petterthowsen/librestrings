//! Circular delay line with 3rd-order Lagrange fractional reads.

#[derive(Clone)]
pub struct DelayLine {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
}

impl DelayLine {
    /// Smallest delay that keeps all four Lagrange taps in the past.
    pub const MIN_DELAY: f32 = 2.0;

    /// Allocates room for delays up to at least `max_delay` samples.
    pub fn new(max_delay: usize) -> Self {
        let len = (max_delay + 4).next_power_of_two();
        Self {
            buf: vec![0.0; len],
            mask: len - 1,
            write: 0,
        }
    }

    pub fn max_delay(&self) -> f32 {
        (self.buf.len() - 3) as f32
    }

    pub fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) & self.mask;
    }

    /// Reads the signal `delay` samples ago; `read(1.0)` is the most recent push.
    /// The delay is clamped to `[MIN_DELAY, max_delay()]`.
    pub fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(Self::MIN_DELAY, self.max_delay());
        let i = d as usize;
        // Taps sit at delays i-1, i, i+1, i+2; t is the read position among them, in [1, 2).
        let t = d - i as f32 + 1.0;
        let x0 = self.tap(i - 1);
        let x1 = self.tap(i);
        let x2 = self.tap(i + 1);
        let x3 = self.tap(i + 2);
        let (t1, t2, t3) = (t - 1.0, t - 2.0, t - 3.0);
        let h0 = -t1 * t2 * t3 * (1.0 / 6.0);
        let h1 = t * t2 * t3 * 0.5;
        let h2 = -t * t1 * t3 * 0.5;
        let h3 = t * t1 * t2 * (1.0 / 6.0);
        h0 * x0 + h1 * x1 + h2 * x2 + h3 * x3
    }

    /// Like [`Self::read`], interpolating linearly: cheaper, and duller
    /// between samples. For paths where that doesn't matter, such as
    /// reflections.
    pub fn read_linear(&self, delay: f32) -> f32 {
        let d = delay.clamp(Self::MIN_DELAY, self.max_delay());
        let i = d as usize;
        let t = d - i as f32;
        let (a, b) = (self.tap(i), self.tap(i + 1));
        a + t * (b - a)
    }

    pub fn clear(&mut self) {
        self.buf.fill(0.0);
    }

    fn tap(&self, delay: usize) -> f32 {
        self.buf[self.write.wrapping_sub(delay) & self.mask]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_delay_is_exact() {
        let mut d = DelayLine::new(64);
        for n in 0..100 {
            d.push(n as f32);
        }
        assert_eq!(d.read(2.0), 98.0);
        assert_eq!(d.read(10.0), 90.0);
    }

    #[test]
    fn fractional_delay_is_exact_for_cubics() {
        let f = |n: f32| 0.001 * n * n * n - 0.2 * n * n + 3.0 * n;
        let mut d = DelayLine::new(64);
        for n in 0..100 {
            d.push(f(n as f32));
        }
        for delay in [2.0, 2.25, 7.5, 31.9] {
            let expected = f(100.0 - delay);
            assert!((d.read(delay) - expected).abs() < 1e-2, "delay {delay}");
        }
    }

    #[test]
    fn linear_read_is_exact_for_lines() {
        let mut d = DelayLine::new(64);
        for n in 0..100 {
            d.push(n as f32);
        }
        for delay in [2.0, 2.25, 7.5, 31.9] {
            assert!(
                (d.read_linear(delay) - (100.0 - delay)).abs() < 1e-4,
                "delay {delay}"
            );
            assert!((d.read_linear(delay) - d.read(delay)).abs() < 1e-3);
        }
    }
}
