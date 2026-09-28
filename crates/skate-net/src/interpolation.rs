//! Rendering-only snapshot timing. Source seconds and local arrival seconds are
//! distinct clocks; no synchronised system clocks or platform transport needed.
use std::collections::VecDeque;
#[derive(Clone, Debug)]
pub struct Sample<T> {
    pub time: f64,
    pub value: T,
}
#[derive(Debug)]
pub struct Buffer<T> {
    pub samples: VecDeque<Sample<T>>,
}
impl<T> Default for Buffer<T> {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
        }
    }
}
impl<T> Buffer<T> {
    pub fn insert(&mut self, time: f64, value: T) -> bool {
        if !time.is_finite() || self.samples.back().is_some_and(|s| time <= s.time) {
            return false;
        }
        self.samples.push_back(Sample { time, value });
        while self.samples.len() > 64 {
            self.samples.pop_front();
        }
        true
    }
    pub fn pair(&self, time: f64) -> Option<(usize, usize, f32)> {
        let first = self.samples.front()?;
        if time <= first.time {
            return Some((0, 0, 0.));
        }
        let upper = self.samples.partition_point(|s| s.time <= time);
        if upper == self.samples.len() {
            return Some((upper - 1, upper - 1, 0.));
        }
        let a = &self.samples[upper - 1];
        let b = &self.samples[upper];
        Some((
            upper - 1,
            upper,
            ((time - a.time) / (b.time - a.time)).clamp(0., 1.) as f32,
        ))
    }
}
#[derive(Default, Debug)]
struct Delivery {
    last: Option<(f64, f64)>,
    gaps: VecDeque<f64>,
    jitter: f64,
}
impl Delivery {
    fn observe(&mut self, source: f64, local: f64) {
        if let Some((s, l)) = self.last {
            if source <= s {
                return;
            }
            let gap = source - s;
            self.gaps.push_back(gap.clamp(0.01, 2.));
            while self.gaps.len() > 16 {
                self.gaps.pop_front();
            }
            self.jitter += (((local - l) - gap).abs().min(1.) - self.jitter) * 0.1;
        }
        self.last = Some((source, local));
    }
    fn interval(&self, default: f64) -> f64 {
        let mut gaps: Vec<_> = self.gaps.iter().copied().collect();
        gaps.sort_by(f64::total_cmp);
        if gaps.is_empty() {
            default
        } else {
            gaps[gaps.len() / 2]
        }
    }
}
#[derive(Default, Debug)]
pub struct Clock {
    delivery: [Delivery; 2],
    cursor: Option<f64>,
    local: Option<f64>,
    drift: f64,
    pub delay: f64,
    pub underruns: u64,
    starving: bool,
    loopback: bool,
}
impl Clock {
    pub fn for_connection(loopback: bool) -> Self {
        Self {
            loopback,
            ..Self::default()
        }
    }
    pub fn observe(&mut self, stream: usize, source: f64, local: f64) {
        if stream < 2 && source.is_finite() && local.is_finite() {
            self.delivery[stream].observe(source, local);
        }
    }
    pub fn step(&mut self, local: f64) -> Option<f64> {
        if !local.is_finite() {
            return self.cursor;
        }
        let head = self
            .delivery
            .iter()
            .filter_map(|d| d.last.map(|(s, _)| s))
            .reduce(f64::min)?;
        let interval = self.delivery[0]
            .interval(0.05)
            .max(self.delivery[1].interval(if self.loopback { 0.05 } else { 0.1 }));
        let jitter = self.delivery.iter().map(|d| d.jitter).fold(0., f64::max);
        self.delay = if self.loopback {
            (1.25 * interval + 2. * jitter).clamp(0.06, 0.15)
        } else {
            (1.5 * interval + 2. * jitter).clamp(0.15, 2.5)
        };
        let dt = self.local.map_or(0., |last| (local - last).max(0.));
        self.local = Some(local);
        let Some(cursor) = self.cursor else {
            self.cursor = Some(head - self.delay);
            return self.cursor;
        };
        // A suspended render loop resumes from a fresh buffer, never replays seconds of backlog.
        if dt > 0.5 || head - cursor > self.delay + if self.loopback { 0.25 } else { 0.75 } {
            self.cursor = Some(head - self.delay);
            self.drift = 0.;
            self.starving = false;
            return self.cursor;
        }
        let error = head - cursor - self.delay;
        self.drift += (error - self.drift) * (dt / (0.35 + dt));
        let threshold = (interval * 0.35).max(0.02);
        let speed = if self.drift > threshold {
            1.08
        } else if self.drift < -threshold {
            0.92
        } else {
            1.
        };
        let next = cursor + dt * speed;
        if next > head {
            if !self.starving {
                self.underruns += 1;
            }
            self.starving = true;
        } else {
            self.starving = false;
        }
        self.cursor = Some(next.min(head).max(cursor));
        self.cursor
    }
}
/// Limited cubic Hermite position interpolation. Tangents come from root
/// positions at their own timestamps, not velocities of an unrelated COM.
/// Limiting prevents curves overshooting the two endpoints at stops/impacts.
pub fn position(buffer: &Buffer<[f32; 3]>, time: f64) -> Option<[f32; 3]> {
    let (a, b, u) = buffer.pair(time)?;
    let x = &buffer.samples[a];
    let y = &buffer.samples[b];
    if a == b {
        return Some(x.value);
    }
    let before = &buffer.samples[a.saturating_sub(1)];
    let after = buffer.samples.get(b + 1).unwrap_or(y);
    let dt = (y.time - x.time) as f32;
    Some(std::array::from_fn(|i| {
        let delta = y.value[i] - x.value[i];
        if delta.abs() < 1e-7 {
            return x.value[i];
        }
        let mut m0 = (y.value[i] - before.value[i]) / ((y.time - before.time) as f32) * dt;
        let mut m1 = (after.value[i] - x.value[i]) / ((after.time - x.time) as f32) * dt;
        if m0 / delta < 0. {
            m0 = 0.;
        }
        if m1 / delta < 0. {
            m1 = 0.;
        }
        let length = ((m0 / delta).powi(2) + (m1 / delta).powi(2)).sqrt();
        if length > 3. {
            m0 *= 3. / length;
            m1 *= 3. / length;
        }
        let u2 = u * u;
        let u3 = u2 * u;
        (2. * u3 - 3. * u2 + 1.) * x.value[i]
            + (u3 - 2. * u2 + u) * m0
            + (-2. * u3 + 3. * u2) * y.value[i]
            + (u3 - u2) * m1
    }))
}
