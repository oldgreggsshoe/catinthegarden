//! Graphics quality: one setting that trades detail for frame time.
//!
//! Chosen at startup from `PLANET_QUALITY` or else the saved setting. Scenario runs and unit tests
//! ignore the saved file so recorded results never depend on a player's
//! preference; they still honour `PLANET_QUALITY` and the individual
//! `PLANET_MAX_ACTIVE_CHUNKS` / `PLANET_OCEAN_COARSE_*` overrides.
//! F5 cycles the setting live and saves it for the next launch.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    High,
    Medium,
    Low,
}

impl Quality {
    pub fn name(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "high" => Some(Self::High),
            "medium" => Some(Self::Medium),
            "low" => Some(Self::Low),
            _ => None,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::High => Self::Medium,
            Self::Medium => Self::Low,
            Self::Low => Self::High,
        }
    }

    /// Terrain/ocean leaf budget: the main triangle-count lever.
    pub fn chunk_budget(self) -> usize {
        match self {
            Self::High => crate::planet::DEFAULT_MAX_ACTIVE_CHUNKS,
            Self::Medium => 192,
            Self::Low => 128,
        }
    }

    /// Interior thinning of far ocean chunks (their boundary ring is kept).
    pub fn ocean_coarse_step(self) -> usize {
        match self {
            Self::High | Self::Medium => 2,
            Self::Low => 4,
        }
    }

    /// Ocean chunks whose cells project below this many pixels are thinned.
    pub fn ocean_coarse_cell_pixels(self) -> f64 {
        match self {
            Self::High | Self::Medium => 8.0,
            Self::Low => 16.0,
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("planet").join("settings.json"))
}

fn load_saved() -> Option<Quality> {
    let text = std::fs::read_to_string(settings_path()?).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    Quality::parse(value.get("quality")?.as_str()?)
}

fn save(quality: Quality) {
    let Some(path) = settings_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = serde_json::json!({ "quality": quality.name() });
    if let Err(error) = std::fs::write(&path, body.to_string()) {
        tracing::warn!(target: "planet::quality", %error, "could not save the quality setting");
    }
}

fn reads_saved_setting() -> bool {
    !cfg!(test) && !std::env::args().any(|argument| argument == "--scenario")
}

fn active_slot() -> &'static AtomicU8 {
    static ACTIVE: OnceLock<AtomicU8> = OnceLock::new();
    ACTIVE.get_or_init(|| {
        let initial = std::env::var("PLANET_QUALITY")
            .ok()
            .and_then(|value| Quality::parse(&value))
            .or_else(|| reads_saved_setting().then(load_saved).flatten())
            .unwrap_or(Quality::High);
        AtomicU8::new(initial as u8)
    })
}

/// The quality in force now (the startup choice until F5 changes it).
pub fn active() -> Quality {
    match active_slot().load(Ordering::Relaxed) {
        1 => Quality::Medium,
        2 => Quality::Low,
        _ => Quality::High,
    }
}

/// F5: step High -> Medium -> Low, save it for later launches, and return the
/// new value for the caller to apply to the running renderer.
pub fn cycle() -> Quality {
    let next = active().next();
    active_slot().store(next as u8, Ordering::Relaxed);
    save(next);
    tracing::info!(target: "planet::quality", quality = next.name(), "quality changed");
    next
}

/// One line for the debug overlay.
pub fn status_text() -> String {
    format!("Quality: {} (F5 cycles, saved)", active().name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_step_down_in_cost_and_cycle() {
        assert_eq!(
            Quality::High.chunk_budget(),
            crate::planet::DEFAULT_MAX_ACTIVE_CHUNKS
        );
        assert!(Quality::High.chunk_budget() > Quality::Medium.chunk_budget());
        assert!(Quality::Medium.chunk_budget() > Quality::Low.chunk_budget());
        assert_eq!(Quality::High.next().next().next(), Quality::High);
        for quality in [Quality::High, Quality::Medium, Quality::Low] {
            assert_eq!(Quality::parse(quality.name()), Some(quality));
        }
        assert_eq!(Quality::parse("ultra"), None);
        // Unit tests never read a player's saved file.
        assert_eq!(active(), Quality::High);
    }

    #[test]
    fn the_saved_setting_round_trips_through_the_config_file() {
        let directory = std::env::temp_dir().join(format!("planet-quality-{}", std::process::id()));
        // Only this test touches XDG_CONFIG_HOME.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &directory) };
        assert_eq!(load_saved(), None);
        save(Quality::Low);
        assert_eq!(load_saved(), Some(Quality::Low));
        std::fs::write(settings_path().unwrap(), "not json").unwrap();
        assert_eq!(load_saved(), None);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
