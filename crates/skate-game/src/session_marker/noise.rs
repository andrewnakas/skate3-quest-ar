//! Native six-word generator82A8AF10 and noise texture827ED7E8.
//! Independent stream: the original shares this generator with other render work.
pub(super) struct Noise {
    words: [u32; 6],
    pub phases: [f32; 2],
    pub scroll: [f32; 4],
}
impl Default for Noise {
    fn default() -> Self {
        Self {
            words: [
                4063039062, 2284922601, 3324304687, 117621916, 2654289789, 1876900708,
            ],
            phases: [0.; 2],
            scroll: [0.; 4],
        }
    }
}
impl Noise {
    fn next(&mut self) -> u32 {
        let mut carry = 0u64;
        for i in (0..5).rev() {
            let sum = self.words[i] as u64 + self.words[i + 1] as u64 + carry;
            self.words[i] = sum as u32;
            carry = sum >> 32;
        }
        for i in (0..6).rev() {
            self.words[i] = self.words[i].wrapping_add(1);
            if self.words[i] != 0 {
                break;
            }
        }
        self.words[0]
    }
    pub fn texture(&mut self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(64 * 64 * 4);
        for _ in 0..64 * 64 * 4 {
            bytes.push(if (self.next() >> 8) & 255 > 128 {
                255
            } else {
                0
            });
        }
        bytes
    }
    pub fn advance(&mut self) {
        for (phase, delta) in self.phases.iter_mut().zip([0.045f32, 0.06]) {
            *phase += delta;
            if *phase >= 4. {
                *phase -= 4.;
            }
        }
        //827EE03C calls four times, then writes fourth, third, second, first.
        self.scroll = std::array::from_fn(|_| (self.next() & 65535) as f32 / 65536.);
        self.scroll.reverse();
    }
    pub fn weights(phase: f32) -> [f32; 4] {
        let mut weights = [0.; 4];
        let index = phase as usize;
        weights[index % 4] = 1. - phase.fract();
        weights[(index + 1) % 4] = phase.fract();
        weights
    }
}
