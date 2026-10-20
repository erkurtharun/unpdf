//! `DeviceCMYK` to sRGB, the way PDFium converts it.
//!
//! The naive complement (`R = (1 - C)(1 - K)`) paints pure cyan as (0, 255, 255) and a full black
//! ink as (0, 0, 0); a printed page looks like neither, and every renderer that reads PDFs
//! approximates the press instead. PDFium samples the CMYK cube on a 9 x 9 x 9 x 9 grid of sRGB
//! values (from an Adobe press profile) and interpolates between the nearest samples; this is
//! that conversion, integer arithmetic included, so a color comes out as it does there. The
//! table is `cmyk_srgb_data` (BSD-3-Clause, see THIRD-PARTY-NOTICES).

use super::cmyk_srgb_data::CMYK_TO_SRGB;

fn sample(c: usize, m: usize, y: usize, k: usize) -> [i32; 3] {
    CMYK_TO_SRGB[729 * c + 81 * m + 9 * y + k].map(i32::from)
}

/// sRGB for the inks `c`, `m`, `y`, `k` (0 none, 255 full).
pub(crate) fn cmyk_to_rgb(c: u8, m: u8, y: u8, k: u8) -> [u8; 3] {
    let fix = [c, m, y, k].map(|v| i32::from(v) << 8);
    let nearest = fix.map(|f| ((f + 4096) >> 13) as usize);
    let start = sample(nearest[0], nearest[1], nearest[2], nearest[3]);
    let mut out = start.map(|v| v << 8);
    for axis in 0..4 {
        // The neighbouring grid line on the side the value lies on (the one below it when it
        // is the nearest one itself, except at the top).
        let mut other = (fix[axis] >> 13) as usize;
        if other == nearest[axis] {
            other = if other == 8 { other - 1 } else { other + 1 };
        }
        let mut index = nearest;
        index[axis] = other;
        let neighbour = sample(index[0], index[1], index[2], index[3]);
        let rate =
            (fix[axis] - ((nearest[axis] as i32) << 13)) * (nearest[axis] as i32 - other as i32);
        for (o, (s, n)) in out.iter_mut().zip(start.iter().zip(&neighbour)) {
            *o += (s - n) * rate / 32;
        }
    }
    out.map(|v| (v.max(0) >> 8).min(255) as u8)
}

/// [`cmyk_to_rgb`] for inks in `0..=1`: sRGB in `0..=1`.
pub(crate) fn cmyk_to_rgb_unit(c: f32, m: f32, y: f32, k: f32) -> [f32; 3] {
    // The float just under one half, as PDFium rounds a unit ink to a byte.
    const ROUND: f32 = 0.499_999_97;
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + ROUND) as u8;
    cmyk_to_rgb(byte(c), byte(m), byte(y), byte(k)).map(|v| f32::from(v) / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corners_of_the_cube_are_the_table_samples() {
        assert_eq!(cmyk_to_rgb(0, 0, 0, 0), [255, 255, 255]);
        // Full black ink is not pure black on the press: (35, 31, 32), as PDFium (and MuPDF)
        // give it.
        assert_eq!(cmyk_to_rgb(0, 0, 0, 255), [35, 31, 32]);
        // Full cyan is not (0, 255, 255) but the process cyan.
        assert_eq!(cmyk_to_rgb(255, 0, 0, 0), [0, 174, 239]);
    }

    #[test]
    fn a_value_between_two_samples_lies_between_them() {
        let a = cmyk_to_rgb(0, 0, 0, 0);
        let b = cmyk_to_rgb(0, 0, 0, 32); // the next grid line, 1/8
        let mid = cmyk_to_rgb(0, 0, 0, 16);
        for i in 0..3 {
            assert!(
                mid[i] <= a[i] && mid[i] >= b[i],
                "{mid:?} not within {a:?}..{b:?}"
            );
        }
    }

    #[test]
    fn more_ink_never_lightens() {
        for k in 0..255u8 {
            let (lo, hi) = (cmyk_to_rgb(40, 20, 90, k), cmyk_to_rgb(40, 20, 90, k + 1));
            assert!(
                hi.iter().zip(lo).all(|(h, l)| *h <= l + 1),
                "k={k}: {lo:?} -> {hi:?}"
            );
        }
    }
}
