//! Graphics quality: one setting that trades detail for frame time.
//!
//! Chosen once at startup (the chunk budget sizes GPU buffers), from
//! `PLANET_QUALITY` or else the saved setting. Scenario runs and unit tests
//! ignore the saved file so recorded results never depend on a player's
//! preference; they still honour `PLANET_QUALITY` and the individual
//! `PLANET_MAX_ACTIVE_CHUNKS` / `PLANET_OCEAN_COARSE_*` overrides.
//! F5 cycles the saved setting; it takes effect on the next launch.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

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
            Self::High => 256,
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

/// The quality this run was started with.
pub fn active() -> Quality {
    static ACTIVE: OnceLock<Quality> = OnceLock::new();
    *ACTIVE.get_or_init(|| {
        std::env::var("PLANET_QUALITY")
            .ok()
            .and_then(|value| Quality::parse(&value))
            .or_else(|| reads_saved_setting().then(load_saved).flatten())
            .unwrap_or(Quality::High)
    })
}

fn pending_slot() -> &'static Mutex<Quality> {
    static PENDING: OnceLock<Mutex<Quality>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(active()))
}

/// F5: step the saved setting High -> Medium -> Low and store it for the next
/// launch. Returns the new saved value.
pub fn cycle_saved() -> Quality {
    let mut pending = pending_slot().lock().unwrap_or_else(|e| e.into_inner());
    *pending = pending.next();
    save(*pending);
    tracing::info!(target: "planet::quality", saved = pending.name(), active = active().name(), "quality setting changed; applies on restart");
    *pending
}

/// One line for the debug overlay.
pub fn status_text() -> String {
    let pending = *pending_slot().lock().unwrap_or_else(|e| e.into_inner());
    if pending == active() {
        format!("Quality: {} (F5 cycles; applies on restart)", active().name())
    } else {
        format!(
            "Quality: {} now, {} after restart (F5)",
            active().name(),
            pending.name()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_step_down_in_cost_and_cycle() {
        assert_eq!(Quality::High.chunk_budget(), crate::planet::DEFAULT_MAX_ACTIVE_CHUNKS);
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
