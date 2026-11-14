//! OKLab (Björn Ottosson, 2020) on linear sRGB: a perceptual space whose hue and chroma
//! behave, for HSL and colour grading. `cbrt` keeps negative (out-of-gamut) light defined.

#[inline]
pub fn to_oklab(c: [f32; 3]) -> [f32; 3] {
    let l = 0.412_221_46 * c[0] + 0.536_332_55 * c[1] + 0.051_445_995 * c[2];
    let m = 0.211_903_5 * c[0] + 0.680_699_5 * c[1] + 0.107_396_96 * c[2];
    let s = 0.088_302_46 * c[0] + 0.281_718_85 * c[1] + 0.629_978_7 * c[2];
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

#[inline]
pub fn from_oklab(p: [f32; 3]) -> [f32; 3] {
    let l = p[0] + 0.396_337_78 * p[1] + 0.215_803_76 * p[2];
    let m = p[0] - 0.105_561_346 * p[1] - 0.063_854_17 * p[2];
    let s = p[0] - 0.089_484_18 * p[1] - 1.291_485_5 * p[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

/// Chroma and hue in degrees (0–360) of an OKLab colour.
#[inline]
pub fn lch(p: [f32; 3]) -> (f32, f32) {
    (p[1].hypot(p[2]), p[2].atan2(p[1]).to_degrees().rem_euclid(360.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for c in [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.8, 0.1, 0.05], [0.02, 0.3, 0.9], [1.4, -0.05, 0.2]] {
            let back = from_oklab(to_oklab(c));
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() < 1e-4, "{c:?} → {back:?}");
            }
        }
    }

    #[test]
    fn white_is_neutral_and_bright() {
        let w = to_oklab([1.0; 3]);
        assert!((w[0] - 1.0).abs() < 1e-3 && w[1].abs() < 1e-3 && w[2].abs() < 1e-3);
    }

    #[test]
    fn primaries_have_their_known_hues() {
        // Published OKLCh hues of the sRGB primaries: red 29°, green 142°, blue 264°.
        for (c, h) in [([1.0, 0.0, 0.0], 29.2), ([0.0, 1.0, 0.0], 142.5), ([0.0, 0.0, 1.0], 264.1)] {
            let (_, hue) = lch(to_oklab(c));
            assert!((hue - h).abs() < 1.0, "{c:?}: {hue}");
        }
    }
}
