//! Relative planning costs, not currency or a claim of optimal civil engineering.
//! Length/ongoing travel cost applies to BOTH surface roads and bores, so a long
//! switchback is not free merely because its local gradients are gentle.
use crate::road_network::Ground;

const TRAVEL: f64 = 0.35;
// Prefer a covered shortcut over even a moderate mountain detour. Keep a
// premium over ordinary surface road; feasibility/cover is checked separately.
const LAND_BORE: f64 = 2.0;
const REGIONAL_LAND_BORE: f64 = 1.25;
const SEA_BORE: f64 = 6.0;

pub fn bore(cover: f64, sea: bool) -> f64 {
    bore_at_cost(cover, sea, LAND_BORE)
}
fn bore_at_cost(cover: f64, sea: bool, land_cost: f64) -> f64 {
    TRAVEL + if sea { SEA_BORE } else { land_cost } + cover.max(0.0) / 2000.0
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
    survey_edge_at_cost(length, a, b, REGIONAL_LAND_BORE)
}

/// Keep the coarse planetary gateway topology stable. It has no engineered
/// alignment/roof cover yet; the stronger preference belongs to regional
/// routing, not speculative long connections through unverified gateways.
pub fn backbone_edge(length: f64, a: Ground, b: Ground) -> f64 {
    survey_edge_at_cost(length, a, b, LAND_BORE)
}
fn survey_edge_at_cost(length: f64, a: Ground, b: Ground, land_cost: f64) -> f64 {
    let rise = (b.height - a.height).abs();
    let sea = a.ocean || b.ocean;
    let tunnel = length * bore_at_cost(rise.max(-a.height.min(b.height)), sea, land_cost);
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
    fn regional_tunnel_preference_beats_a_fifty_percent_surface_detour() {
        let land = Ground {
            height: 1000.0,
            ocean: false,
        };
        let ridge = Ground {
            height: 1100.0,
            ocean: false,
        };
        let preferred_shortcut = 2.0 * survey_edge(200.0, land, ridge);
        let detour = survey_edge(600.0, land, land);
        assert!(preferred_shortcut < detour);
        assert!(2.0 * backbone_edge(200.0, land, ridge) > detour);
        assert!(survey_edge(400.0, land, land) < preferred_shortcut);
        assert!((bore(100.0, true) - 6.4).abs() < 1e-10);
    }
    #[test]
    fn regional_tunnel_preference_preserves_conservative_backbone_and_sea_costs() {
        let a = Ground {
            height: 1000.0,
            ocean: false,
        };
        let b = Ground {
            height: 2500.0,
            ocean: false,
        };
        assert!(survey_edge(1000.0, a, b) < backbone_edge(1000.0, a, b));
        assert!((backbone_edge(1000.0, a, b) - 3100.0).abs() < 1e-8);
        let sea = Ground {
            height: -100.0,
            ocean: true,
        };
        assert_eq!(survey_edge(1000.0, a, sea), backbone_edge(1000.0, a, sea));
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
