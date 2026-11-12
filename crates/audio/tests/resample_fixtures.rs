//! Resampler fixtures: 44.1/48 kHz, arbitrary block boundaries, final flush.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use dictation_audio::resample::StreamResampler;

fn sine(rate: u32, hz: f32, secs: f32) -> Vec<f32> {
    let n = (rate as f32 * secs) as usize;
    (0..n)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
        .collect()
}

/// Deterministic pseudo-random block sizes (LCG), 1..=4096.
fn blocks(len: usize, seed: u64) -> Vec<usize> {
    let mut s = seed;
    let mut out = vec![];
    let mut left = len;
    while left > 0 {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let b = ((s >> 33) as usize % 4096 + 1).min(left);
        out.push(b);
        left -= b;
    }
    out
}

fn resample(input: &[f32], rate: u32, sizes: &[usize]) -> Vec<f32> {
    let mut r = StreamResampler::new(rate).unwrap();
    let mut out = vec![];
    let mut pos = 0;
    for &b in sizes {
        r.push(&input[pos..pos + b], &mut out).unwrap();
        pos += b;
    }
    r.flush(&mut out).unwrap();
    assert!(r.flush(&mut out).is_err(), "flush only once");
    out
}

fn zero_crossings(x: &[f32]) -> usize {
    x.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

#[test]
fn rates_and_block_boundaries() {
    for rate in [44_100u32, 48_000, 16_000, 22_050, 96_000] {
        let input = sine(rate, 440.0, 1.5);
        let expected = (input.len() as f64 * 16_000.0 / f64::from(rate)).round() as usize;
        let a = resample(&input, rate, &[input.len()]);
        let b = resample(&input, rate, &blocks(input.len(), 7));
        let c = resample(&input, rate, &vec![1; input.len()][..]);
        for (name, out) in [("one-shot", &a), ("random", &b), ("1-sample", &c)] {
            assert_eq!(out.len(), expected, "{rate} Hz {name}");
        }
        // Block partitioning must not change the output.
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() < 1e-4, "{rate}: partition-dependent output");
        }
        // 440 Hz → ~880 crossings/s preserved; skip edges.
        let mid = &a[1600..a.len() - 1600];
        let hz = zero_crossings(mid) as f32 / 2.0 / (mid.len() as f32 / 16_000.0);
        assert!((hz - 440.0).abs() < 3.0, "{rate}: measured {hz} Hz");
        // Startup delay trimmed: signal present right from the start (after the first ms).
        assert!(
            a[16..160].iter().any(|s| s.abs() > 0.1),
            "{rate}: leading delay not trimmed"
        );
    }
}

#[test]
fn rejects_absurd_rates() {
    assert!(StreamResampler::new(0).is_err());
    assert!(StreamResampler::new(1_000_000).is_err());
}
