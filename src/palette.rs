pub type Rgb = (u8, u8, u8);

const BASE: [Rgb; 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

pub fn pal_index_rgb(idx: u8) -> Rgb {
    let i = idx as usize;
    if i < 16 {
        return BASE[i];
    }
    if i < 232 {
        let k = i - 16;
        let lvl = |v: usize| -> u8 {
            if v == 0 {
                0
            } else {
                (55 + 40 * v) as u8
            }
        };
        return (lvl(k / 36), lvl((k % 36) / 6), lvl(k % 6));
    }
    let g = 8 + 10 * (i - 232);
    (g as u8, g as u8, g as u8)
}

pub fn spec_index(spec: &str) -> Option<u8> {
    let t = spec.trim();
    if let Ok(v) = t.parse::<u32>() {
        if (30..=37).contains(&v) {
            return Some((v - 30) as u8);
        }
        if (90..=97).contains(&v) {
            return Some((v - 82) as u8);
        }
        return None;
    }
    let mut it = t.split(';');
    match (it.next(), it.next(), it.next(), it.next()) {
        (Some("38"), Some("5"), Some(n), None) => n.parse::<u8>().ok(),
        _ => None,
    }
}

pub fn spec_rgb(spec: &str) -> Option<Rgb> {
    spec_index(spec).map(pal_index_rgb)
}

pub fn lerp_rgb(lo: Rgb, hi: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t + 0.5) as u8;
    (mix(lo.0, hi.0), mix(lo.1, hi.1), mix(lo.2, hi.2))
}

pub fn cube_idx_nearest(rgb: Rgb) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let sh = |v: u8| -> u32 {
        let mut best = 0u32;
        let mut dist = u32::MAX;
        for (i, l) in LEVELS.iter().enumerate() {
            let d = u32::from(v.abs_diff(*l));
            if d < dist {
                dist = d;
                best = i as u32;
            }
        }
        best
    };
    (16 + 36 * sh(rgb.0) + 6 * sh(rgb.1) + sh(rgb.2)) as u8
}

pub fn ramp_rgb(lo: &str, hi: &str, t: f64) -> Option<Rgb> {
    let (a, b) = (spec_index(lo)?, spec_index(hi)?);
    if a == b || t <= 0.0 {
        return Some(pal_index_rgb(a));
    }
    if t >= 1.0 {
        return Some(pal_index_rgb(b));
    }
    Some(lerp_rgb(pal_index_rgb(a), pal_index_rgb(b), t as f32))
}

pub fn ramp_spec(lo: &str, hi: &str, t: f64) -> Option<String> {
    let (a, b) = (spec_index(lo)?, spec_index(hi)?);
    if a == b {
        return Some(lo.trim().to_string());
    }
    if t <= 0.0 {
        return Some(lo.trim().to_string());
    }
    if t >= 1.0 {
        return Some(hi.trim().to_string());
    }
    let rgb = lerp_rgb(pal_index_rgb(a), pal_index_rgb(b), t as f32);
    Some(format!("38;5;{}", cube_idx_nearest(rgb)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_map_to_palette_indexes() {
        assert_eq!(spec_index("34"), Some(4));
        assert_eq!(spec_index("36"), Some(6));
        assert_eq!(spec_index("90"), Some(8));
        assert_eq!(spec_index("97"), Some(15));
        assert_eq!(spec_index("38;5;196"), Some(196));
        assert_eq!(spec_index("1;36"), None);
        assert_eq!(spec_index("#ff0000"), None);
        assert_eq!(spec_index("38;5;256"), None);
    }

    #[test]
    fn base_pair_ramps_past_two_colors() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..=64 {
            let spec = ramp_spec("34", "36", i as f64 / 64.0).unwrap();
            seen.insert(spec);
        }
        assert!(seen.len() > 2, "expected intermediate colors, got {:?}", seen);
        assert!(seen.contains("34") && seen.contains("36"));
    }

    #[test]
    fn rgb_ramp_uses_endpoint_colors() {
        assert_eq!(ramp_rgb("34", "36", 0.0), Some(BASE[4]));
        assert_eq!(ramp_rgb("34", "36", 1.0), Some(BASE[6]));
        assert_eq!(ramp_rgb("38;5;196", "38;5;196", 0.5), Some(BASE[9]));
        assert_eq!(ramp_rgb("#ff0000", "34", 0.5), None);
    }

    #[test]
    fn cube_lookup_preserves_cube_colors() {
        assert_eq!(cube_idx_nearest(pal_index_rgb(221)), 221);
        assert_eq!(cube_idx_nearest(pal_index_rgb(32)), 32);
        assert_eq!(cube_idx_nearest(pal_index_rgb(16)), 16);
    }
}
