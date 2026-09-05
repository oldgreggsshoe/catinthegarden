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
    Crater, EJECTA_EXTENT, MOON_DATUM_METERS, POLAR_ICE_DEPTH_FRACTION,
    POLAR_ICE_LATITUDE_SINE_FULL, POLAR_ICE_LATITUDE_SINE_START, runtime,
};

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
    let (window_sin, window_cos) = catalogue.field_window_radians().sin_cos();
    source.push_str(&format!(
        "const MOON_FIELD_WINDOW_SIN: f32 = {window_sin:.9};\n\
         const MOON_FIELD_WINDOW_COS: f32 = {window_cos:.9};\n\
         const MOON_EJECTA_EXTENT: f32 = {EJECTA_EXTENT:.4};\n\
         const MOON_POLAR_ICE_DEPTH_FRACTION: f32 = {POLAR_ICE_DEPTH_FRACTION:.4};\n\
         const MOON_POLAR_ICE_LATITUDE_SINE_START: f32 = {POLAR_ICE_LATITUDE_SINE_START:.4};\n\
         const MOON_POLAR_ICE_LATITUDE_SINE_FULL: f32 = {POLAR_ICE_LATITUDE_SINE_FULL:.4};\n"
    ));
    source
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
        assert_eq!(generated.matches("    vec4<f32>(").count(), catalogue.len());
        assert_eq!(generated.matches("    vec3<f32>(").count(), catalogue.len());
        assert!(generated.contains(&format!(
            "const MOON_BASIN_COUNT: u32 = {}u;",
            catalogue.basins().len()
        )));
        assert!(generated.contains(&format!(
            "const MOON_FIELD_COUNT: u32 = {}u;",
            catalogue.field().len()
        )));
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
