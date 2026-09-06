//! Emitting the moon's crater catalogue into the shader.
//!
//! The catalogue itself lives in `catinthegarden_coretypes::moon`, because the
//! baker needs the same one: a moon with a bake and a moon without must be the
//! same body, and two copies of the profile would be a divergence waiting to
//! happen. What is here is only the part the renderer owns — turning the
//! catalogue into WGSL constants.
//!
//! Generated rather than hand-mirrored. `OCEAN_WAVE_TABLE` is hand-copied with
//! a test policing every literal; hundreds of craters would make that a
//! divergence waiting to happen, and generating removes the need for the test
//! rather than adding a second one.
//!
//! **Only the runtime catalogue is emitted.** The baked one is eleven thousand
//! craters, which is not a shader constant — it is tiles. The shader's copy is
//! the placeholder terrain shown when the moon has no bake.

use catinthegarden_coretypes::moon::{
    Crater, EJECTA_EXTENT, MOON_DATUM_METERS, MOON_PLANET_SKY_DIRECTION, MOON_PLANETSHINE_COLOUR,
    MOON_PLANETSHINE_FRACTION, baked, runtime,
};

/// How many impacts carry the surface's *appearance* into the shader.
///
/// Separate from the ones that carry its shape, and far fewer. Height comes
/// from the bake, where 600,176 craters cost nothing per frame; brightness has
/// to be evaluated per fragment, and only the large ones are visible as
/// markings anyway. On the real Moon a handful of young craters -- Tycho,
/// Copernicus, Kepler -- account for nearly all of the visible ray structure.
/// 96, and the ceiling is a measured cliff rather than a budget. On this GPU
/// 128 markings cost nothing at all -- 16.8ms against a 16.6ms baseline -- and
/// 160 cost 130.7ms. That is not a slope, it is the dynamically indexed const
/// arrays falling out of whatever the driver keeps them in, so the number to
/// respect is the edge and not the average. 96 leaves room under it.
///
/// No great loss: the real Moon has perhaps a dozen ray systems worth the name.
const ALBEDO_CRATER_COUNT: usize = 96;

/// How far a bright halo reaches past the rim, as a multiple of the rim radius.
/// The ejecta *blanket* stops at 1.35; the brightening runs further, because
/// what makes a young crater conspicuous is finer material thrown well beyond
/// the blanket it built.
const HALO_EXTENT: f64 = 3.2;

/// And how far the rays run. Tycho's reach most of a lunar radius -- about
/// twenty times its own. Kept shorter here so they read as rays rather than as
/// a global wash.
const RAY_EXTENT: f64 = 14.0;

/// Hard ceiling on how far any marking reaches, in radians.
///
/// This is a *cost* bound, and without it the whole thing is unusable: fourteen
/// rim radii of a 0.30 rad basin is 4.2 radians, more than pi, so its cutoff
/// cosine saturates at -1 and rejects nothing. Every pixel then ran an `acos`,
/// an `atan2` and a `pow` for every large marking, and a 54-second replay had
/// not finished after fifty minutes.
///
/// At 0.35 rad a marking covers about 3% of the sphere, so of 192 markings
/// roughly six reach any given fragment. It does clip the reach of the very
/// largest basins' rays, which is a real loss and the price of the bound.
const MAX_MARKING_REACH_RADIANS: f64 = 0.35;

/// The largest impacts, with what the shader needs to draw their markings.
///
/// Sorted by size, so raising or lowering the count changes how far down the
/// size ladder markings appear rather than which craters have them.
fn albedo_craters() -> Vec<(Crater, f64)> {
    let mut craters: Vec<Crater> = baked().iter().copied().collect();
    craters.sort_by(|a, b| b.angular_radius.total_cmp(&a.angular_radius));
    craters.truncate(ALBEDO_CRATER_COUNT);
    craters
        .into_iter()
        .enumerate()
        .map(|(index, crater)| {
            // A per-crater phase, so every ray system points somewhere
            // different. Real ones are set by the impact angle and the target's
            // structure; this only has to avoid every crater sharing an axis.
            let phase = (index as f64 * 2.399_963_23 + 0.7).sin() * std::f64::consts::PI;
            (crater, phase)
        })
        .collect()
}

pub fn wgsl_constants() -> String {
    let catalogue = runtime();
    let basin_count = catalogue.basins().len();
    let field_count = catalogue.field().len();
    let total = catalogue.len();
    let mut source =
        String::from("// Generated from coretypes::moon via moon.rs. Do not edit here.\n");
    source.push_str(&format!(
        "// {total} impacts: the basins are always tested, the field is windowed.\n\
         const MOON_BASIN_COUNT: u32 = {basin_count}u;\n\
         const MOON_FIELD_COUNT: u32 = {field_count}u;\n\
         const MOON_DATUM_METERS: f32 = {MOON_DATUM_METERS:.1};\n"
    ));
    emit_catalogue(&mut source, "MOON_BASINS", catalogue.basins());
    emit_catalogue(&mut source, "MOON_FIELD", catalogue.field());
    emit_markings(&mut source, &albedo_craters());
    let (window_sin, window_cos) = catalogue.field_window_radians().sin_cos();
    let [sky0, sky1, sky2] = MOON_PLANET_SKY_DIRECTION;
    let [tint0, tint1, tint2] = MOON_PLANETSHINE_COLOUR;
    source.push_str(&format!(
        "const MOON_FIELD_WINDOW_SIN: f32 = {window_sin:.9};\n\
         const MOON_FIELD_WINDOW_COS: f32 = {window_cos:.9};\n\
         const MOON_EJECTA_EXTENT: f32 = {EJECTA_EXTENT:.4};\n\
         const MOON_PLANET_SKY_DIRECTION: vec3<f32> = \
         vec3<f32>({sky0:.9}, {sky1:.9}, {sky2:.9});\n\
         const MOON_PLANETSHINE_FRACTION: f32 = {MOON_PLANETSHINE_FRACTION:.9};\n\
         const MOON_PLANETSHINE_COLOUR: vec3<f32> = \
         vec3<f32>({tint0:.4}, {tint1:.4}, {tint2:.4});\n"
    ));
    source
}

/// The albedo craters: where the markings are, and what they look like.
///
/// These come from the *baked* catalogue rather than the runtime one, which is
/// the whole point -- a bright halo has to sit on the crater that threw it, and
/// the two catalogues are different bodies' worth of impacts.
fn emit_markings(source: &mut String, markings: &[(Crater, f64)]) {
    let count = markings.len();
    source.push_str(&format!(
        "const MOON_MARKING_COUNT: u32 = {count}u;\n\
         // axis.xyz, rim radius in radians.\n\
         const MOON_MARKINGS: array<vec4<f32>, {count}> = array<vec4<f32>, {count}>(\n"
    ));
    for (crater, _) in markings {
        source.push_str(&format!(
            "    vec4<f32>({:.9}, {:.9}, {:.9}, {:.9}),\n",
            crater.axis.x, crater.axis.y, crater.axis.z, crater.angular_radius
        ));
    }
    source.push_str(");\n");
    source.push_str(&format!(
        "// freshness, ray phase, and the cosine cutoffs for halo and rays.\n\
         const MOON_MARKING_TRAITS: array<vec4<f32>, {count}> = array<vec4<f32>, {count}>(\n"
    ));
    for (crater, phase) in markings {
        let halo = (HALO_EXTENT * crater.angular_radius)
            .min(MAX_MARKING_REACH_RADIANS)
            .cos();
        let ray = (RAY_EXTENT * crater.angular_radius)
            .min(MAX_MARKING_REACH_RADIANS)
            .cos();
        source.push_str(&format!(
            "    vec4<f32>({:.6}, {:.6}, {:.9}, {:.9}),\n",
            crater.freshness, phase, halo, ray
        ));
    }
    source.push_str(");\n");
    source.push_str(&format!(
        "const MOON_HALO_EXTENT: f32 = {HALO_EXTENT:.4};\n\
         const MOON_RAY_EXTENT: f32 = {RAY_EXTENT:.4};\n"
    ));
}

/// One catalogue as a pair of WGSL arrays: axes with rim radius, and the
/// depths with the cutoff the loop rejects on.
fn emit_catalogue(source: &mut String, name: &str, craters: &[Crater]) {
    let count = craters.len();
    source.push_str(&format!(
        "// axis.xyz, rim radius in radians.\n\
         const {name}: array<vec4<f32>, {count}> = array<vec4<f32>, {count}>(\n"
    ));
    for crater in craters {
        source.push_str(&format!(
            "    vec4<f32>({:.9}, {:.9}, {:.9}, {:.9}),\n",
            crater.axis.x, crater.axis.y, crater.axis.z, crater.angular_radius
        ));
    }
    source.push_str(");\n");
    source.push_str(&format!(
        "// depth and rim height in metres, then the cosine cutoff.\n\
         const {name}_DEPTHS: array<vec3<f32>, {count}> = array<vec3<f32>, {count}>(\n"
    ));
    for crater in craters {
        source.push_str(&format!(
            "    vec3<f32>({:.4}, {:.4}, {:.9}),\n",
            crater.depth_meters, crater.rim_meters, crater.cosine_cutoff
        ));
    }
    source.push_str(");\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader's copy has to describe the same craters the CPU evaluates,
    /// or the drawn surface and the ground query are different worlds.
    #[test]
    fn the_generated_source_carries_the_runtime_catalogue() {
        let generated = wgsl_constants();
        let catalogue = runtime();
        // Counted per array rather than over the whole file: the markings add
        // their own `vec4` rows, and a single total silently conflates the
        // shape catalogue with the appearance one.
        let rows_in = |name: &str| {
            generated
                .split(&format!("const {name}"))
                .nth(1)
                .and_then(|rest| rest.split(");").next())
                .map(|body| body.matches("    vec4<f32>(").count())
                .unwrap_or(0)
        };
        assert_eq!(
            rows_in("MOON_BASINS:") + rows_in("MOON_FIELD:"),
            catalogue.len(),
            "the shape catalogue reaches the shader whole",
        );
        assert_eq!(generated.matches("    vec3<f32>(").count(), catalogue.len());
        assert_eq!(
            rows_in("MOON_MARKINGS:"),
            ALBEDO_CRATER_COUNT,
            "and so does the appearance catalogue",
        );
        assert!(generated.contains(&format!(
            "const MOON_MARKING_COUNT: u32 = {ALBEDO_CRATER_COUNT}u;"
        )));
        assert!(generated.contains(&format!(
            "const MOON_BASIN_COUNT: u32 = {}u;",
            catalogue.basins().len()
        )));
        assert!(generated.contains(&format!(
            "const MOON_FIELD_COUNT: u32 = {}u;",
            catalogue.field().len()
        )));
    }

    /// The markings must be the craters that are actually there, not the
    /// runtime catalogue's -- a bright halo has to sit on the crater that threw
    /// it, and the two catalogues are different bodies' worth of impacts.
    #[test]
    fn markings_come_from_the_baked_craters_largest_first() {
        let markings = albedo_craters();
        assert_eq!(markings.len(), ALBEDO_CRATER_COUNT);
        for pair in markings.windows(2) {
            assert!(
                pair[0].0.angular_radius >= pair[1].0.angular_radius,
                "markings are ordered by size, so the count sets how far down \
                 the ladder markings appear",
            );
        }
        let largest = baked()
            .iter()
            .map(|crater| crater.angular_radius)
            .fold(0.0_f64, f64::max);
        assert_eq!(markings[0].0.angular_radius, largest);
    }

    /// Every marking's reach must be bounded, or its cutoff cosine saturates at
    /// -1, rejects nothing, and every pixel pays for it. That cost a 54-second
    /// replay fifty minutes.
    #[test]
    fn every_marking_can_reject_a_distant_fragment() {
        for (crater, _) in albedo_craters() {
            let reach = (RAY_EXTENT * crater.angular_radius).min(MAX_MARKING_REACH_RADIANS);
            assert!(
                reach < std::f64::consts::PI - 0.1,
                "a marking reaching {reach} radians cannot reject anything",
            );
        }
    }

    /// The datum is what lifts the whole body above zero. If the shader's copy
    /// drifted from the bake's, the placeholder moon and the baked moon would
    /// sit at different radii.
    #[test]
    fn the_generated_datum_matches_the_one_the_bake_applies() {
        assert!(wgsl_constants().contains(&format!(
            "const MOON_DATUM_METERS: f32 = {MOON_DATUM_METERS:.1};"
        )),);
    }
}
