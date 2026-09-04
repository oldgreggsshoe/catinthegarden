pub mod config;
mod etopo;
mod export;
mod grid;
pub mod terrain;

use std::{io::Write, time::Instant};

use catinthegarden_coretypes::OutmapManifest;

pub use config::BakeConfig;
pub use export::{
    available_tile_keys, refine_existing_outmap, refine_existing_outmap_with_progress,
    sparse_radius_for_level, validate_output,
};
pub use terrain::{MountainVisibilityReport, Terrain};

pub type BakeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Human-facing progress for long-running CLI bakes. Library callers get a
/// disabled reporter by default so tests and embedding applications stay
/// silent unless they opt in.
pub struct BakeProgress {
    enabled: bool,
    stage: &'static str,
    stage_started: Instant,
    last_percent: u8,
}

impl BakeProgress {
    pub fn new() -> Self {
        Self {
            enabled: true,
            stage: "starting",
            stage_started: Instant::now(),
            last_percent: u8::MAX,
        }
    }

    pub(crate) fn disabled() -> Self {
        Self {
            enabled: false,
            stage: "disabled",
            stage_started: Instant::now(),
            last_percent: u8::MAX,
        }
    }

    pub(crate) fn stage(&mut self, stage: &'static str) {
        if !self.enabled {
            return;
        }
        eprintln!("\n[baker] {stage}");
        self.stage = stage;
        self.stage_started = Instant::now();
        self.last_percent = u8::MAX;
    }

    pub(crate) fn begin(&mut self, stage: &'static str, total: usize) {
        self.stage(stage);
        self.update(0, total);
    }

    pub(crate) fn update(&mut self, completed: usize, total: usize) {
        if !self.enabled {
            return;
        }
        let total = total.max(1);
        let completed = completed.min(total);
        let percent = ((completed as u128 * 100) / total as u128) as u8;
        if percent == self.last_percent && completed != total {
            return;
        }
        self.last_percent = percent;
        let elapsed = self.stage_started.elapsed().as_secs_f64();
        let eta = if completed > 0 {
            (elapsed * (total - completed) as f64 / completed as f64).max(0.0)
        } else {
            0.0
        };
        eprint!(
            "\r[baker] {:<28} {:>3}% ({completed}/{total}), ETA {eta:>6.1}s",
            self.stage, percent
        );
        let _ = std::io::stderr().flush();
        if completed == total {
            eprintln!();
        }
    }

    pub(crate) fn done(&mut self) {
        if self.enabled {
            eprintln!("[baker] {} complete", self.stage);
        }
    }
}

impl Default for BakeProgress {
    fn default() -> Self {
        Self::new()
    }
}

pub fn bake(config: &BakeConfig) -> BakeResult<OutmapManifest> {
    let mut progress = BakeProgress::disabled();
    bake_internal(config, false, &mut progress).map(|(manifest, _)| manifest)
}

pub fn bake_with_mountain_coverage(
    config: &BakeConfig,
) -> BakeResult<(OutmapManifest, MountainVisibilityReport)> {
    let mut progress = BakeProgress::disabled();
    bake_internal(config, true, &mut progress)
}

pub fn bake_with_progress(
    config: &BakeConfig,
    report_mountain_coverage: bool,
    progress: &mut BakeProgress,
) -> BakeResult<(OutmapManifest, MountainVisibilityReport)> {
    bake_internal(config, report_mountain_coverage, progress)
}

fn bake_internal(
    config: &BakeConfig,
    report_mountain_coverage: bool,
    progress: &mut BakeProgress,
) -> BakeResult<(OutmapManifest, MountainVisibilityReport)> {
    config
        .validate()
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
    progress.stage("terrain generation");
    let terrain = Terrain::try_generate_with_progress(config, progress)?;
    progress.done();
    let mountain_coverage = if report_mountain_coverage {
        progress.stage("mountain coverage survey");
        let report = terrain.mountain_visibility_coverage();
        progress.done();
        report
    } else {
        Default::default()
    };
    let manifest = export::export_outmap_with_progress(config, &terrain, progress)?;
    progress.stage("validating output");
    validate_output_with_progress(&config.output, progress)?;
    Ok((manifest, mountain_coverage))
}

pub fn validate_output_with_progress(
    output: &std::path::Path,
    progress: &mut BakeProgress,
) -> BakeResult<OutmapManifest> {
    export::validate_output_with_progress(output, progress)
}
