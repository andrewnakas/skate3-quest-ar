//! Point-backed countdowns, 82DA4C28 and module82DA37B0.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointTimer {
    pub points: f32,
    pub expired: bool,
}
impl PointTimer {
    /// Returns the native near-one hold result used by the ground collector.
    pub fn advance(&mut self, dt: f32, drain: f32, scale: f32, hold_near_one: bool) -> bool {
        self.expired = false;
        if self.points > 0.0 {
            let previous = self.points;
            self.points = -((drain * dt) * scale - self.points);
            if hold_near_one && self.points < 1.001 && previous > 1.000001 {
                self.points = previous;
                return true;
            }
            if self.points <= 0.0 {
                self.points = 0.0;
                self.expired = true;
            }
        }
        false
    }
    pub fn credit(&mut self, points: f32, capacity: f32) {
        self.points = (self.points + points).min(capacity);
    }
    pub fn seconds(&self, drain: f32) -> f32 {
        self.points / drain
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComboTimer {
    pub timer: PointTimer,
    pub multiplier: f32,
}
impl Default for ComboTimer {
    fn default() -> Self {
        Self {
            timer: PointTimer::default(),
            multiplier: 1.0,
        }
    }
}
impl ComboTimer {
    /// The multiplier may increase, or decrease when a sufficiently large
    /// new reward authorizes refreshing it. Timer decay alone does not lower it.
    pub fn credit(&mut self, reward: f32, capacity: f32, levels: [(f32, f32); 3], refresh: f32) {
        self.timer.credit(reward, capacity);
        let mut next = 1.0;
        for (threshold, multiplier) in levels {
            if self.timer.points >= threshold {
                next = multiplier;
            }
        }
        if next > self.multiplier || reward > refresh {
            self.multiplier = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_ground_hold_preserves_previous_points_and_expiry_is_an_edge() {
        let mut timer = PointTimer {
            points: 2.0,
            expired: false,
        };
        assert!(timer.advance(0.1, 50.0, 1.0, true));
        assert_eq!(timer.points, 2.0);
        assert!(!timer.advance(0.1, 50.0, 1.0, false));
        assert!(timer.expired);
        timer.advance(0.1, 50.0, 1.0, false);
        assert!(!timer.expired);
    }
    /// What a bounded hold does when its bound runs out, which is what 82DA33E0's counter at
    /// module+112 decides. The hold only arms while a single drain would land the timer under
    /// 1.001 from a `previous` still above 1.000001, so that is a window of exactly one
    /// drain's width: deny the hold once and, save for a sliver barely 0.001 wide, the timer
    /// falls below the arming threshold and can never hold again. That is why a parked line
    /// does expire, and why it can take more than one full window to do it.
    #[test]
    fn a_denied_hold_drops_the_timer_below_the_arming_threshold() {
        let (dt, drain, scale) = (1. / 60., 50.0, 0.2);
        let step = drain * dt * scale;
        let mut timer = PointTimer {
            points: 1.001 + step / 2.,
            expired: false,
        };
        // Armed: the points come straight back and the caller is told the hold took effect.
        let before = timer.points;
        assert!(timer.advance(dt, drain, scale, true));
        assert_eq!(timer.points, before);
        // Denied once, as the counter reaching its bound does.
        assert!(!timer.advance(dt, drain, scale, false));
        assert!(timer.points < 1.000001);
        // From there the hold cannot arm again, whatever the caller asks for.
        assert!(!timer.advance(dt, drain, scale, true));
        for _ in 0..8 {
            timer.advance(dt, drain, scale, true);
        }
        assert_eq!(timer.points, 0.0);
    }
    #[test]
    fn multiplier_uses_credited_points_and_survives_timer_decay() {
        let levels = [(50.0, 1.5), (450.0, 2.0), (800.0, 3.0)];
        let mut combo = ComboTimer::default();
        combo.credit(50.0, 801.0, levels, 799.0);
        assert_eq!(combo.multiplier, 1.5);
        combo.credit(400.0, 801.0, levels, 799.0);
        assert_eq!(combo.multiplier, 2.0);
        combo.credit(350.0, 801.0, levels, 799.0);
        assert_eq!(combo.multiplier, 3.0);
        combo.timer.advance(8.0, 100.0, 1.0, false);
        combo.credit(1.0, 801.0, levels, 799.0);
        assert_eq!(combo.multiplier, 3.0);
    }
}
