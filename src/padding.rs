// Copyright (C) 2026 Toit contributors.
//! Explicit uniform-padding convolution with Gaussian width error.
//!
//! This is conditional on a stated padding prior, not a recovered-text posterior.
//! The omitted Gaussian constant matches the existing point-width score.
use anyhow::{Result, ensure};

#[link(name = "m")]
unsafe extern "C" {
    fn erfc(x: f64) -> f64;
}

const LOG_SQRT_TWO_PI: f64 = 0.918_938_533_204_672_7;

fn log_survival(z: f64) -> f64 {
    debug_assert!(z >= 0.0 && z.is_finite());
    if z < 26.0 {
        // SAFETY: libm erfc accepts every finite double and owns no memory.
        return (0.5 * unsafe { erfc(z * std::f64::consts::FRAC_1_SQRT_2) }).ln();
    }
    // erfc asymptotic expansion (NIST DLMF 7.12.1). Stop before increasing
    // terms: an asymptotic series is not a convergent infinite sum.
    let inv = 1.0 / z / z;
    let mut term: f64 = 1.0;
    let mut series = 1.0;
    for n in 1..=16 {
        let next = -term * (2 * n - 1) as f64 * inv;
        if next.abs() >= term.abs() {
            break;
        }
        series += next;
        term = next;
        if term.abs() < f64::EPSILON * series.abs() {
            break;
        }
    }
    -0.5 * z * z - z.ln() - LOG_SQRT_TWO_PI + series.ln()
}

/// Negative log average Gaussian density over [low, high], with the common
/// log(sqrt(2*pi)) removed. At zero width this is exactly the point-width cost.
/// All inputs are in the same physical units; scaling units adds log(scale).
pub fn uniform_cost(prediction: f64, low: f64, high: f64, sigma: f64) -> Result<f64> {
    ensure!(
        [prediction, low, high, sigma].iter().all(|x| x.is_finite()) && low <= high && sigma > 0.0,
        "Invalid uniform-padding likelihood parameters"
    );
    let width = high - low;
    let a = (low - prediction) / sigma;
    let b = (high - prediction) / sigma;
    let scaled_width = width / sigma;
    let midpoint = a + scaled_width * 0.5;
    ensure!(
        [width, a, b, scaled_width, midpoint]
            .iter()
            .all(|x| x.is_finite()),
        "Uniform-padding standardized bounds overflow"
    );
    let cost = if width == 0.0 {
        0.5 * a * a + sigma.ln()
    } else if scaled_width * (1.0 + midpoint.abs()) < 1e-4 {
        // Midpoint density with second-order averaged-density correction.
        // Avoid subtracting nearly equal CDFs and take the point-width limit
        // smoothly. The fourth-order remainder here is below double precision.
        let correction = ((midpoint * scaled_width).powi(2) - scaled_width.powi(2)) / 24.0;
        0.5 * midpoint * midpoint + sigma.ln() - correction.ln_1p()
    } else {
        let log_mass = if a >= 0.0 {
            let left = log_survival(a);
            left + (-(log_survival(b) - left).exp_m1()).ln()
        } else if b <= 0.0 {
            let left = log_survival(-b);
            left + (-(log_survival(-a) - left).exp_m1()).ln()
        } else {
            (-log_survival(-a).exp() - log_survival(b).exp()).ln_1p()
        };
        -log_mass + width.ln() - LOG_SQRT_TWO_PI
    };
    ensure!(
        cost.is_finite(),
        "Uniform-padding likelihood overflow or unresolved bounds"
    );
    Ok(cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_limit_symmetry_and_units() {
        assert_eq!(
            uniform_cost(9.0, 10.0, 10.0, 2.0).unwrap(),
            0.125 + 2.0_f64.ln()
        );
        for sigma in [0.5, 1.0, 5.0] {
            let center = uniform_cost(100.0, 94.0, 106.0, sigma).unwrap();
            let edge = uniform_cost(106.0, 94.0, 106.0, sigma).unwrap();
            assert!(center < edge);
            assert_eq!(edge, uniform_cost(94.0, 94.0, 106.0, sigma).unwrap());
            let scaled = uniform_cost(200.0, 188.0, 212.0, 2.0 * sigma).unwrap();
            assert!((scaled - center - 2.0_f64.ln()).abs() < 1e-12);
            let point = uniform_cost(101.0, 100.0, 100.0, sigma).unwrap();
            let narrow = uniform_cost(101.0, 100.0 - 1e-8, 100.0 + 1e-8, sigma).unwrap();
            assert!((point - narrow).abs() < 1e-12);
        }
    }

    #[test]
    fn tails_and_quadrature_agree_without_underflow() {
        // Independent composite midpoint quadrature over the padding interval,
        // rescaled by its largest log-density to avoid underflow in the tail.
        let cases: [(f64, f64, f64, f64); 6] = [
            (0., -6., 6., 1.),
            (0., 0., 1., 1.),
            (0., 8., 9., 1.),
            (0., 40., 40.2, 1.),
            (0., 100., 100.1, 1.),
            (10., 9., 11., 0.5),
        ];
        for (prediction, low, high, sigma) in cases {
            let n = 100_000;
            let reference = -0.5 * ((low - prediction) / sigma).powi(2);
            let total: f64 = (0..n)
                .map(|i| {
                    let x = low + (high - low) * (i as f64 + 0.5) / n as f64;
                    (-0.5 * ((x - prediction) / sigma).powi(2) - reference).exp()
                })
                .sum();
            let expected = -(total / n as f64).ln() - reference + sigma.ln();
            assert!((uniform_cost(prediction, low, high, sigma).unwrap() - expected).abs() < 1e-8);
        }
        assert!(uniform_cost(0., 1000., 1001., 1.).unwrap().is_finite());
        assert!(uniform_cost(0., f64::MAX, f64::MAX, 1.).is_err());
        assert!(uniform_cost(0., 1., 0., 1.).is_err());
        assert!(uniform_cost(0., 0., 1., 0.).is_err());
        assert!(uniform_cost(f64::NAN, 0., 1., 1.).is_err());
    }
}
