//! Recorded thunder for the sea synthesiser to play, one clip per strike.
//!
//! The clips are CC0 recordings cut from `assets/sounds/thunder` (sources in
//! its `CREDITS.md`): single thunder events with short fades, mono, peak
//! normalised to 0.9, Ogg Vorbis. They are sorted by how fast each one rises
//! to its loudest, which is what distance does to thunder: a close strike
//! cracks, a far one is a slow low roll. `crack_*` rise within half a second,
//! `mid_*` within about a second, `roll_*` take longer.

use std::path::{Path, PathBuf};

/// Where the clips live, relative to the working directory (the repository
/// root under `cargo run`), with the source tree as a fallback.
const CLIP_DIRECTORY: &str = "assets/sounds/thunder/clips";

/// The kinds of thunder, nearest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Crack = 0,
    Mid = 1,
    Roll = 2,
}

/// Strikes nearer than this (after a little jitter) play a crack...
const CRACK_WITHIN_METERS: f32 = 2_000.0;
/// ...nearer than this a mid peal, and anything farther a roll.
const MID_WITHIN_METERS: f32 = 5_000.0;

/// Every clip is played so that its loudest two seconds have this RMS. Peak
/// normalisation left them 2.5x apart in loudness (a spiky crack or a quiet
/// far roll was lost under a storm's sea and wind next to a dense one).
const CLIP_LOUDNESS: f32 = 0.15;
const LOUDNESS_WINDOW_SECONDS: f32 = 2.0;

/// One decoded clip: mono samples at the clip's own rate, and the gain that
/// brings it to `CLIP_LOUDNESS`.
pub(crate) struct Clip {
    pub(crate) samples: Vec<f32>,
    pub(crate) rate: f32,
    pub(crate) level: f32,
}

/// RMS of the loudest `LOUDNESS_WINDOW_SECONDS` of `samples`, in 0.1 s steps.
fn loudest_rms(samples: &[f32], rate: f32) -> f32 {
    let hop = ((0.1 * rate) as usize).max(1);
    let energies: Vec<f32> = samples
        .chunks(hop)
        .map(|chunk| chunk.iter().map(|value| value * value).sum::<f32>() / chunk.len() as f32)
        .collect();
    let window = ((LOUDNESS_WINDOW_SECONDS / 0.1) as usize).clamp(1, energies.len().max(1));
    energies
        .windows(window)
        .map(|run| run.iter().sum::<f32>() / window as f32)
        .fold(0.0_f32, f32::max)
        .sqrt()
}

pub(crate) struct ThunderClips {
    pub(crate) clips: Vec<Clip>,
    /// Indices into `clips`, by `Kind`.
    kinds: [Vec<usize>; 3],
}

impl ThunderClips {
    /// Decodes every `crack_*`, `mid_*` and `roll_*` clip it can find. None
    /// when there are none, so the synthesiser keeps its own thump.
    pub(crate) fn load() -> Option<Self> {
        let directory = [
            PathBuf::from(CLIP_DIRECTORY),
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(CLIP_DIRECTORY),
        ]
        .into_iter()
        .find(|path| path.is_dir())?;
        let mut names: Vec<PathBuf> = std::fs::read_dir(&directory)
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.extension().is_some_and(|extension| extension == "ogg"))
            .collect();
        names.sort();
        let mut clips = Vec::new();
        let mut kinds: [Vec<usize>; 3] = Default::default();
        for path in names {
            let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("");
            let kind = if stem.starts_with("crack") {
                Kind::Crack
            } else if stem.starts_with("mid") {
                Kind::Mid
            } else if stem.starts_with("roll") {
                Kind::Roll
            } else {
                continue;
            };
            match decode(&path) {
                Ok(clip) if !clip.samples.is_empty() => {
                    kinds[kind as usize].push(clips.len());
                    clips.push(clip);
                }
                Ok(_) => {}
                Err(error) => tracing::warn!(
                    target: "planet::sound",
                    path = %path.display(),
                    %error,
                    "thunder clip not loaded"
                ),
            }
        }
        if clips.is_empty() {
            return None;
        }
        Some(Self { clips, kinds })
    }

    /// The clip for a strike `distance_meters` away. `jitter` and `pick` are
    /// uniform 0-1 draws; `avoid` is the clip played last, not repeated if
    /// there is another of the kind. Falls back to the nearest kind that has
    /// clips.
    pub(crate) fn choose(&self, distance_meters: f32, jitter: f32, pick: f32, avoid: Option<usize>) -> usize {
        let kind = kind_for_distance(distance_meters * (0.8 + 0.4 * jitter));
        let order = match kind {
            Kind::Crack => [Kind::Crack, Kind::Mid, Kind::Roll],
            Kind::Mid => [Kind::Mid, Kind::Crack, Kind::Roll],
            Kind::Roll => [Kind::Roll, Kind::Mid, Kind::Crack],
        };
        let pool = order
            .iter()
            .map(|kind| &self.kinds[*kind as usize])
            .find(|pool| !pool.is_empty())
            .expect("load() returns None without clips");
        let candidates: Vec<usize> = pool
            .iter()
            .copied()
            .filter(|index| pool.len() == 1 || Some(*index) != avoid)
            .collect();
        candidates[((pick * candidates.len() as f32) as usize).min(candidates.len() - 1)]
    }

    #[cfg(test)]
    pub(crate) fn kind_of(&self, index: usize) -> Kind {
        [Kind::Crack, Kind::Mid, Kind::Roll]
            .into_iter()
            .find(|kind| self.kinds[*kind as usize].contains(&index))
            .expect("every clip has a kind")
    }
}

pub(crate) fn kind_for_distance(distance_meters: f32) -> Kind {
    if distance_meters < CRACK_WITHIN_METERS {
        Kind::Crack
    } else if distance_meters < MID_WITHIN_METERS {
        Kind::Mid
    } else {
        Kind::Roll
    }
}

/// Air takes the highs out of distant thunder: the cutoff (Hz) of the low-pass
/// a strike `distance_meters` away is heard through, from 9 kHz at 300 m down
/// to 500 Hz at 9 km, evenly in log frequency.
pub(crate) fn thunder_cutoff_hz(distance_meters: f32) -> f32 {
    let t = ((distance_meters - 300.0) / 8_700.0).clamp(0.0, 1.0);
    9_000.0 * (500.0_f32 / 9_000.0).powf(t)
}

fn decode(path: &Path) -> Result<Clip, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut reader = lewton::inside_ogg::OggStreamReader::new(std::io::BufReader::new(file))
        .map_err(|error| error.to_string())?;
    let channels = usize::from(reader.ident_hdr.audio_channels).max(1);
    let rate = reader.ident_hdr.audio_sample_rate as f32;
    let mut samples = Vec::new();
    while let Some(packet) = reader
        .read_dec_packet_itl()
        .map_err(|error| error.to_string())?
    {
        for frame in packet.chunks(channels) {
            let sum: f32 = frame.iter().map(|&sample| f32::from(sample)).sum();
            samples.push(sum / (channels as f32 * 32_768.0));
        }
    }
    let loudness = loudest_rms(&samples, rate);
    let level = if loudness > 1.0e-6 { CLIP_LOUDNESS / loudness } else { 0.0 };
    Ok(Clip { samples, rate, level })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_clips_load_by_kind() {
        let clips = ThunderClips::load().expect("assets/sounds/thunder/clips is in the repository");
        assert_eq!(
            clips.kinds.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![2, 5, 4],
            "two cracks, five mid peals, four rolls"
        );
        for clip in &clips.clips {
            let seconds = clip.samples.len() as f32 / clip.rate;
            assert!((4.0..25.0).contains(&seconds), "{seconds} s");
            let peak = clip.samples.iter().fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            assert!((0.5..=1.0).contains(&peak), "peak {peak}");
            let scaled: Vec<f32> = clip.samples.iter().map(|value| value * clip.level).collect();
            assert!((loudest_rms(&scaled, clip.rate) - CLIP_LOUDNESS).abs() < 1.0e-3);
        }
    }

    #[test]
    fn near_strikes_crack_and_far_ones_roll() {
        let clips = ThunderClips::load().expect("clips");
        for jitter in [0.0, 0.5, 1.0] {
            for pick in [0.0, 0.3, 0.99] {
                assert_eq!(clips.kind_of(clips.choose(300.0, jitter, pick, None)), Kind::Crack);
                assert_eq!(clips.kind_of(clips.choose(3_500.0, jitter, pick, None)), Kind::Mid);
                assert_eq!(clips.kind_of(clips.choose(9_000.0, jitter, pick, None)), Kind::Roll);
            }
        }
        // The clip just played is not picked again.
        for pick in [0.0, 0.5, 0.99] {
            let first = clips.choose(9_000.0, 0.5, pick, None);
            assert_ne!(clips.choose(9_000.0, 0.5, pick, Some(first)), first);
        }
    }

    #[test]
    fn farther_thunder_is_duller() {
        assert!((thunder_cutoff_hz(300.0) - 9_000.0).abs() < 1.0);
        assert!((thunder_cutoff_hz(9_000.0) - 500.0).abs() < 1.0);
        assert!(thunder_cutoff_hz(2_000.0) > thunder_cutoff_hz(5_000.0));
    }
}
