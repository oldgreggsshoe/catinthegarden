//! Relative planning costs, not currency or a claim of optimal civil engineering.
//! Length/ongoing travel cost applies to BOTH surface roads and bores, so a long
//! switchback is not free merely because its local gradients are gentle.
use crate::road_network::Ground;

const TRAVEL: f64 = 0.35;
// Prefer a covered shortcut over even a moderate mountain detour. Keep a
// premium over ordinary surface road; feasibility/cover is checked separately.
const LAND_BORE: f64 = 1.25;
const SEA_BORE: f64 = 6.0;

pub fn bore(cover: f64, sea: bool) -> f64 {
    TRAVEL + if sea { SEA_BORE } else { LAND_BORE } + cover.max(0.0) / 2000.0
}

pub fn at_height(ground: Ground, deck: f64) -> f64 {
    let cover = ground.height - deck;
    if ground.ocean || cover > 19.0 {
        bore(cover, ground.ocean)
    } else {
        TRAVEL
            + 1.0
            + 0.002 * (cover + crate::road::CUT_FLOOR_DEPTH).max(0.0).powi(2)
            + 0.01 * (deck - ground.height - 8.0).max(0.0)
    }
}

/// Coarse topology cannot fit the deck yet. Price both a terrain-following road
/// and an estimated bore instead of treating steep ground as an infinite wall.
/// Subsequent shared profiles still verify grades, curves and actual roof cover.
pub fn survey_edge(length: f64, a: Ground, b: Ground) -> f64 {
    let rise = (b.height - a.height).abs();
    let sea = a.ocean || b.ocean;
    let tunnel = length * bore(rise.max(-a.height.min(b.height)), sea);
    if sea {
        return tunnel;
    }
    let excess_grade = (rise / length.max(0.001) - 0.08).max(0.0);
    let surface = length * (1.0 + TRAVEL + 25.0 * excess_grade * excess_grade) + 2.0 * rise;
    surface.min(tunnel)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn land_bore_beats_a_twenty_five_percent_surface_detour() {
        let land = Ground {
            height: 1000.0,
            ocean: false,
        };
        let direct_bore = 1200.0 * at_height(land, 900.0);
        let detour = 1500.0 * at_height(land, 1008.0);
        assert!(
            direct_bore < detour,
            "bore {direct_bore}, surface detour {detour}"
        );
        // An equally short ordinary road remains preferable; sea engineering
        // is not discounted and deep cover still contributes to construction.
        assert!(1200.0 * at_height(land, 1008.0) < direct_bore);
        assert!((bore(100.0, true) - 6.4).abs() < 1e-10);
    }
    #[test]
    fn bores_cost_more_per_metre_but_can_beat_a_long_detour() {
        let land = Ground {
            height: 1000.0,
            ocean: false,
        };
        assert!(at_height(land, 1008.0) < at_height(land, 900.0));
        assert!(1200.0 * at_height(land, 900.0) < 3000.0 * at_height(land, 1008.0));
        assert!(bore(100.0, true) > bore(100.0, false));
        assert!(bore(2000.0, false) > bore(100.0, false));
    }
}
