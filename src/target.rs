pub const BANDS_HZ: [f32; 25] = [
    63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0, 500.0, 630.0, 800.0, 1000.0,
    1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0, 8000.0, 10000.0, 12500.0,
    16000.0,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Voice {
    Male,
    Female,
}

/// Byrne 1994 LTASS, 1/3-octave dB.
pub fn ltass_db(voice: Voice) -> [f32; 25] {
    match voice {
        Voice::Male => [
            38.6, 43.5, 54.4, 57.7, 56.8, 58.2, 59.7, 60.0, 62.4, 62.6, 60.6, 55.7, 53.1, 53.7,
            52.3, 48.7, 48.9, 47.0, 46.0, 44.4, 43.3, 42.4, 41.9, 39.8, 40.4,
        ],
        Voice::Female => [
            37.0, 36.0, 37.5, 40.1, 53.4, 62.2, 60.9, 58.1, 61.7, 61.7, 60.4, 58.0, 54.3, 52.3,
            51.7, 48.8, 47.3, 46.7, 45.3, 44.6, 45.2, 44.9, 45.0, 42.8, 41.1,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_spot_checks() {
        assert_eq!(ltass_db(Voice::Male)[12], 53.1);
        assert_eq!(ltass_db(Voice::Female)[5], 62.2);
        assert_eq!((BANDS_HZ[0], BANDS_HZ[24]), (63.0, 16000.0));
    }
}
