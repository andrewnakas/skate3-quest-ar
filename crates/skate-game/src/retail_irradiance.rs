//! Spatial SH samples from RX2 0xEB0024. TU3 827E6AC8 selects a nearest sample.
use bevy::prelude::*;

#[derive(Clone)]
pub(crate) struct Probe {
    pub position: Vec3,
    pub sh: [Vec4; 9],
}
pub(crate) struct ProbeGroup {
    min: Vec3,
    max: Vec3,
    probes: Vec<Probe>,
}
#[derive(Default)]
pub(crate) struct Irradiance {
    groups: Vec<ProbeGroup>,
    recent: Vec<Probe>,
}
impl Irradiance {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.get(..4) != Some(b"SHP1") {
            return Err("Invalid irradiance header".into());
        }
        let mut at = 4;
        fn word(bytes: &[u8], at: &mut usize) -> Result<u32, String> {
            let raw = bytes.get(*at..*at + 4).ok_or("Truncated irradiance")?;
            *at += 4;
            Ok(u32::from_le_bytes(raw.try_into().unwrap()))
        }
        let count = word(bytes, &mut at)?;
        let mut groups = Vec::new();
        for _ in 0..count {
            let n = word(bytes, &mut at)?;
            let mut probes = Vec::new();
            let mut min = Vec3::splat(f32::INFINITY);
            let mut max = Vec3::splat(f32::NEG_INFINITY);
            for _ in 0..n {
                let mut v = [0.; 39];
                for x in &mut v {
                    *x = f32::from_bits(word(bytes, &mut at)?);
                }
                word(bytes, &mut at)?; // Runtime linked-list word, not lighting.
                if !v.iter().all(|v| v.is_finite()) {
                    return Err("Non-finite irradiance".into());
                }
                let position = Vec3::from_slice(&v[36..39]);
                min = min.min(position);
                max = max.max(position);
                probes.push(Probe {
                    position,
                    sh: std::array::from_fn(|i| Vec4::from_slice(&v[i * 4..i * 4 + 4])),
                });
            }
            if !probes.is_empty() {
                groups.push(ProbeGroup { min, max, probes });
            }
        }
        if at != bytes.len() {
            return Err("Trailing irradiance bytes".into());
        }
        Ok(Self {
            groups,
            recent: Vec::new(),
        })
    }
    pub fn sample(&mut self, position: Vec3, fallback: [Vec4; 9]) -> [Vec4; 9] {
        // Native six-entry cache is accepted inside 2.5 metres of a sample.
        if let Some((i, _)) = self
            .recent
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.position.distance_squared(position)))
            .filter(|(_, d)| *d < 6.25)
            .min_by(|a, b| a.1.total_cmp(&b.1))
        {
            let probe = self.recent.remove(i);
            let sh = probe.sh;
            self.recent.insert(0, probe);
            return sh;
        }
        let mut best = None;
        let mut distance = f32::MAX;
        for group in &self.groups {
            if position.cmplt(group.min - Vec3::splat(15.)).any()
                || position.cmpgt(group.max + Vec3::splat(15.)).any()
            {
                continue;
            }
            for p in &group.probes {
                let delta = (p.position - position).abs();
                if delta.x > 15. || delta.z > 15. {
                    continue;
                }
                let d = p.position.distance_squared(position);
                if d < distance {
                    distance = d;
                    best = Some(p.clone());
                }
            }
        }
        match best {
            Some(probe) => {
                let sh = probe.sh;
                self.recent.insert(0, probe);
                self.recent.truncate(6);
                sh
            }
            None => fallback,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spatial_selection_and_outside_fallback() {
        let mut b = b"SHP1".to_vec();
        b.extend(1u32.to_le_bytes());
        b.extend(2u32.to_le_bytes());
        for (x, colour) in [(0f32, 0.2f32), (10., 0.8)] {
            let mut v = [0f32; 40];
            v[0] = colour;
            v[36] = x;
            for f in v {
                b.extend(f.to_le_bytes());
            }
        }
        let mut data = Irradiance::parse(&b).unwrap();
        assert_eq!(data.sample(Vec3::ZERO, [Vec4::ZERO; 9])[0].x, 0.2);
        assert_eq!(data.sample(Vec3::X * 9., [Vec4::ZERO; 9])[0].x, 0.8);
        assert_eq!(data.sample(Vec3::X * 100., [Vec4::ONE; 9]), [Vec4::ONE; 9]);
        assert!(Irradiance::parse(&b[..b.len() - 1]).is_err());
    }
    #[test]
    fn cache_retains_native_two_point_five_metre_hysteresis() {
        let a = Probe {
            position: Vec3::ZERO,
            sh: [Vec4::ZERO; 9],
        };
        let b = Probe {
            position: Vec3::X * 3.,
            sh: [Vec4::ONE; 9],
        };
        let mut data = Irradiance {
            groups: vec![ProbeGroup {
                min: Vec3::ZERO,
                max: b.position,
                probes: vec![a.clone(), b],
            }],
            recent: vec![a],
        };
        // B is closer, but A remains selected within the native cache radius.
        assert_eq!(
            data.sample(Vec3::X * 2., [Vec4::splat(2.); 9]),
            [Vec4::ZERO; 9]
        );
        assert_eq!(
            data.sample(Vec3::X * 2.5, [Vec4::splat(2.); 9]),
            [Vec4::ONE; 9]
        );
    }
}
