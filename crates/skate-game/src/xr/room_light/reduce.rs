//! A camera frame (YUV 4:2:0) reduced to Roomtone's grid: each cell's mean
//! linear colour and the share of its pixels that clipped.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

/// The grid's side, cells.
pub(crate) const GRID: usize = 32;
/// Every `STEP`th pixel of a cell, each way.
const STEP: usize = 5;
/// Luma from here up has clipped: a light brighter than the exposure can show.
const CLIPPED: u8 = 250;

/// One plane of an image as the camera hands it over.
#[derive(Clone, Copy)]
pub(crate) struct Plane<'a> {
    pub data: &'a [u8],
    pub row_stride: usize,
    pub pixel_stride: usize,
}

impl Plane<'_> {
    fn at(&self, row: usize, column: usize) -> Option<u8> {
        self.data.get(row * self.row_stride + column * self.pixel_stride).copied()
    }
}

/// sRGB's decoding, by byte.
pub(crate) fn linear_table() -> [f32; 256] {
    std::array::from_fn(|i| {
        let c = i as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    })
}

/// Fills `cells` (`GRID * GRID * 4`: r, g, b, clipped) from a
/// `width` x `height` frame: full-range BT.601 YCbCr, as camera YUV is,
/// its chroma planes at half size.
pub(crate) fn reduce(y: Plane, u: Plane, v: Plane, width: usize, height: usize, linear: &[f32; 256], cells: &mut [f32]) {
    let (cell_w, cell_h) = (width / GRID, height / GRID);
    let byte = |c: f32| c.round().clamp(0.0, 255.0) as usize;
    for gy in 0..GRID {
        for gx in 0..GRID {
            let (mut sum, mut count, mut clipped) = ([0.0f32; 3], 0u32, 0u32);
            for row in (gy * cell_h + STEP / 2..(gy + 1) * cell_h).step_by(STEP) {
                for column in (gx * cell_w + STEP / 2..(gx + 1) * cell_w).step_by(STEP) {
                    let (Some(l), Some(cb), Some(cr)) = (y.at(row, column), u.at(row / 2, column / 2), v.at(row / 2, column / 2))
                    else {
                        continue;
                    };
                    clipped += (l >= CLIPPED) as u32;
                    let (l, cb, cr) = (l as f32, cb as f32 - 128.0, cr as f32 - 128.0);
                    sum[0] += linear[byte(l + 1.402 * cr)];
                    sum[1] += linear[byte(l - 0.344136 * cb - 0.714136 * cr)];
                    sum[2] += linear[byte(l + 1.772 * cb)];
                    count += 1;
                }
            }
            let cell = &mut cells[(gy * GRID + gx) * 4..][..4];
            let n = count.max(1) as f32;
            cell[0] = sum[0] / n;
            cell[1] = sum[1] / n;
            cell[2] = sum[2] / n;
            cell[3] = clipped as f32 / n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_mean_linear_colour_and_the_share_clipped() {
        // 320 x 320: grey 128 on the left half, clipped white on the right,
        // the top-left cell red; a row stride wider than the image.
        let (w, h, stride) = (320usize, 320usize, 352usize);
        let mut y = vec![0u8; stride * h];
        let (mut u, mut v) = (vec![128u8; stride * h / 2], vec![128u8; stride * h / 2]);
        for row in 0..h {
            for column in 0..w {
                y[row * stride + column] = if column < w / 2 { 128 } else { 255 };
            }
        }
        // Red (255, 0, 0): Y 76, Cb 85, Cr 255; interleaved chroma (pixel stride 2).
        for row in 0..10 {
            for column in 0..10 {
                y[row * stride + column] = 76;
            }
        }
        for row in 0..5 {
            for column in 0..5 {
                u[row * stride + column * 2] = 85;
                v[row * stride + column * 2] = 255;
            }
        }
        let mut cells = vec![0.0f32; GRID * GRID * 4];
        let linear = linear_table();
        reduce(
            Plane { data: &y, row_stride: stride, pixel_stride: 1 },
            Plane { data: &u, row_stride: stride, pixel_stride: 2 },
            Plane { data: &v, row_stride: stride, pixel_stride: 2 },
            w,
            h,
            &linear,
            &mut cells,
        );
        let cell = |gx: usize, gy: usize| &cells[(gy * GRID + gx) * 4..][..4];
        let grey = linear[128];
        assert!(cell(5, 5).iter().zip([grey, grey, grey, 0.0]).all(|(a, b)| (a - b).abs() < 1e-4), "{:?}", cell(5, 5));
        assert!(cell(20, 9).iter().zip([1.0, 1.0, 1.0, 1.0]).all(|(a, b)| (a - b).abs() < 1e-4), "{:?}", cell(20, 9));
        let red = cell(0, 0);
        assert!(red[0] > 0.95 && red[1] < 0.01 && red[2] < 0.01 && red[3] == 0.0, "{red:?}");
    }
}
