use std::f32::consts::TAU;

pub fn sine(sr: f32, hz: f32, amp: f32, secs: f32) -> Vec<f32> {
    (0..(sr * secs) as usize)
        .map(|i| amp * (TAU * hz * i as f32 / sr).sin())
        .collect()
}

/// Deterministic uniform noise in ±amp (LCG, seed 12345).
pub fn noise(n: usize, amp: f32) -> Vec<f32> {
    let mut s: u32 = 12345;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            amp * ((s >> 8) as f32 / (1u32 << 23) as f32 - 1.0)
        })
        .collect()
}
