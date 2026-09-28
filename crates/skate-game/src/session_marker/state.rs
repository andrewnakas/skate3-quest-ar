//! PlayerUI::UpdateSessionMarker 82898FC8. Time advances in native UI ticks.
#[derive(Default, Debug)]
pub(super) struct Hold {
    elapsed: f32,
    fired: bool,
    tail: u8,
}

#[derive(Default, Debug, PartialEq)]
pub(super) struct Step {
    pub progress: f32,
    pub relocate: bool,
}

impl Hold {
    pub fn cancel(&mut self) {
        *self = Self::default();
    }

    pub fn update(&mut self, held: bool, usable: bool, distance: f32, ready: bool) -> Step {
        let mut out = Step::default();
        if !held {
            self.elapsed = 0.;
            self.fired = false;
        } else if !self.fired && usable {
            self.elapsed += f32::from_bits(0x3c88_8889);
            //828992C8: within half a metre there is no effect or relocation.
            if distance > 0.5 && distance.is_finite() {
                let duration = duration(distance);
                if ready && self.elapsed > duration {
                    self.fired = true;
                    self.tail = 3;
                    out.relocate = true;
                }
                out.progress = (self.elapsed / duration).clamp(0., 1.);
            }
        }
        //828994F0 also executes in the relocation tick.
        if self.tail > 0 {
            out.progress = 1.;
            self.tail -= 1;
        }
        out
    }
}

pub(super) fn duration(distance: f32) -> f32 {
    if distance <= 100. {
        0.2
    } else if distance >= 1000. {
        1.
    } else {
        distance.mul_add(f32::from_bits(0x3a69_0453), f32::from_bits(0x3de3_8e39))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn release_cancels_and_completed_hold_does_not_repeat() {
        let mut hold = Hold::default();
        for _ in 0..6 {
            assert!(!hold.update(true, true, 20., true).relocate);
        }
        assert_eq!(hold.update(false, true, 20., true), Step::default());
        for _ in 0..11 {
            assert!(!hold.update(true, true, 20., true).relocate);
        }
        let mut count = 0;
        for _ in 0..120 {
            count += usize::from(hold.update(true, true, 20., true).relocate);
        }
        assert_eq!(count, 1);
        assert_eq!(hold.update(true, true, 20., true).progress, 0.);
    }
    #[test]
    fn unset_nearby_and_blocked_returns() {
        let mut hold = Hold::default();
        for _ in 0..100 {
            assert_eq!(hold.update(true, false, 10., true), Step::default());
        }
        for _ in 0..100 {
            assert_eq!(hold.update(true, true, 0.5, true), Step::default());
        }
        assert!(!hold.update(true, true, 10., false).relocate);
        assert!(hold.update(true, true, 10., true).relocate);
        assert_eq!(hold.update(false, true, 10., true).progress, 1.);
        assert_eq!(hold.update(false, true, 10., true).progress, 1.);
        assert_eq!(hold.update(false, true, 10., true).progress, 0.);
    }
    #[test]
    fn distance_ramp_matches_native_endpoints() {
        assert_eq!(duration(1.), 0.2);
        assert_eq!(duration(100.), 0.2);
        assert!((duration(550.) - 0.6).abs() < 0.000001);
        assert_eq!(duration(1000.), 1.);
        assert_eq!(duration(2000.), 1.);
    }
    #[test]
    fn distance_changes_the_actual_relocation_tick() {
        for (distance, expected_tick) in [(100., 13), (550., 36), (1000., 61)] {
            let mut hold = Hold::default();
            let tick = (1..=120).find(|_| hold.update(true, true, distance, true).relocate);
            assert_eq!(tick, Some(expected_tick), "distance {distance}");
        }
    }
}
