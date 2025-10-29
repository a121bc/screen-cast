use std::f64::consts::TAU;

#[derive(Clone, Debug)]
pub struct Rng64 {
    state: u64,
}

impl Rng64 {
    pub fn new(seed: u64) -> Self {
        let init = if seed == 0 { 0x0123_4567_89AB_CDEF } else { seed };
        Self { state: init }
    }

    pub fn uniform(&mut self) -> f64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let value = (self.state >> 11) as f64;
        value / (1u64 << 53) as f64
    }

    pub fn gaussian(&mut self, mean: f64, std_dev: f64) -> f64 {
        if std_dev <= f64::EPSILON {
            return mean;
        }

        let u1 = self.uniform().clamp(1e-12, 0.999_999_999_999);
        let u2 = self.uniform();
        let mag = (-2.0 * u1.ln()).sqrt();
        let z0 = mag * (TAU * u2).cos();
        mean + z0 * std_dev
    }
}
