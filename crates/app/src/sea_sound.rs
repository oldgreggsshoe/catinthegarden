//! The sound of the sea, synthesised: no recordings.
//!
//! Three layers of filtered noise. Breaking waves are bursts that start bright
//! (the crash) and sweep down into a hiss (the wash), arriving at random and
//! more often as the sea gets up. Small splashes lap against the hull in a
//! slight sea. Under both, a low steady roar. In a small sea the breaks are
//! separate, with quiet between; as the waves grow they come faster, longer
//! and louder and the roar rises, until in a storm they overlap into one
//! nearly stable roar.
//!
//! The game sets three numbers each frame (`SeaSound::set`): how rough the sea
//! is, how near the water the listener is, and how far under it. The audio
//! thread eases toward them, so nothing clicks.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Overall loudness; `CATINGARDEN_SOUND_VOLUME` scales it (0-2).
const VOLUME: f32 = 0.5;
/// Seconds the audio thread takes to follow a change in the sea.
const PARAMETER_EASE_SECONDS: f32 = 0.6;
/// Breaking waves per second in a slight sea and in a full storm.
const BREAK_RATE_CALM: f32 = 0.35;
const BREAK_RATE_STORM: f32 = 4.0;
/// Hull laps per second in a flat calm (none in a storm).
const LAP_RATE_CALM: f32 = 2.5;
/// Most sounds at once; a storm's overlapping breaks sit well under it.
const MAX_BREAKS: usize = 24;
const MAX_LAPS: usize = 8;
/// Loudness of the low roar at full storm; in a calm it is a faint floor.
const ROAR_GAIN: f32 = 1.1;
const ROAR_FLOOR: f32 = 0.06;
/// Distance above the water (m) at which the sea is half as loud.
pub const HALF_LOUDNESS_HEIGHT_METERS: f64 = 40.0;

/// Loudness of the sea for a listener this far above the water (m): full at
/// the surface, half at `HALF_LOUDNESS_HEIGHT_METERS`, faint from the air.
pub fn loudness_at_height(height_meters: f64) -> f32 {
    (1.0 / (1.0 + height_meters.max(0.0) / HALF_LOUDNESS_HEIGHT_METERS)) as f32
}

fn volume() -> f32 {
    std::env::var("CATINGARDEN_SOUND_VOLUME")
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map_or(1.0, |value| value.clamp(0.0, 2.0))
        * VOLUME
}

/// `CATINGARDEN_SOUND=0` keeps the game silent.
fn enabled() -> bool {
    !matches!(
        std::env::var("CATINGARDEN_SOUND")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("0" | "off" | "false")
    )
}

#[derive(Default)]
struct Shared {
    roughness: AtomicU32,
    level: AtomicU32,
    muffle: AtomicU32,
}

fn store(slot: &AtomicU32, value: f32) {
    slot.store(value.to_bits(), Ordering::Relaxed);
}

fn load(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.load(Ordering::Relaxed))
}

pub struct SeaSound {
    shared: Arc<Shared>,
    _stream: Option<cpal::Stream>,
}

impl SeaSound {
    /// No output at all: replays and tests.
    pub fn silent() -> Self {
        Self {
            shared: Arc::new(Shared::default()),
            _stream: None,
        }
    }

    /// Opens the default output device. Without one (or with
    /// `CATINGARDEN_SOUND=0`) the game carries on silently.
    pub fn start() -> Self {
        let shared = Arc::new(Shared::default());
        let stream = if enabled() {
            match open_stream(Arc::clone(&shared)) {
                Ok(stream) => Some(stream),
                Err(error) => {
                    tracing::warn!(target: "catinthegarden::sound", %error, "no sea sound");
                    None
                }
            }
        } else {
            None
        };
        Self {
            shared,
            _stream: stream,
        }
    }

    /// `roughness` 0 (slight sea) to 1 (storm); `level` 0-1 how near the water
    /// the listener is (`loudness_at_height`, 0 away from the sea); `muffle`
    /// 0 in air, 1 under the water.
    pub fn set(&self, roughness: f32, level: f32, muffle: f32) {
        store(&self.shared.roughness, roughness.clamp(0.0, 1.0));
        store(&self.shared.level, level.clamp(0.0, 1.0));
        store(&self.shared.muffle, muffle.clamp(0.0, 1.0));
    }
}

fn open_stream(shared: Arc<Shared>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no default output device".to_owned())?;
    let config = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, config, shared),
        cpal::SampleFormat::I16 => build::<i16>(&device, config, shared),
        cpal::SampleFormat::U16 => build::<u16>(&device, config, shared),
        cpal::SampleFormat::I32 => build::<i32>(&device, config, shared),
        other => Err(format!("unsupported sample format {other}")),
    }?;
    stream.play().map_err(|error| error.to_string())?;
    tracing::info!(target: "catinthegarden::sound", "sea sound playing");
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let mut synth = SeaSynth::new(config.sample_rate as f32, 0x5ea5_0001);
    let volume = volume();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                synth.set_targets(
                    load(&shared.roughness),
                    load(&shared.level) * volume,
                    load(&shared.muffle),
                );
                for frame in data.chunks_mut(channels) {
                    let [left, right] = synth.next_frame();
                    for (channel, sample) in frame.iter_mut().enumerate() {
                        let value = match (channels, channel % 2) {
                            (1, _) => 0.5 * (left + right),
                            (_, 0) => left,
                            _ => right,
                        };
                        *sample = T::from_sample(value);
                    }
                }
            },
            |error| tracing::warn!(target: "catinthegarden::sound", %error, "sea sound stream"),
            None,
        )
        .map_err(|error| error.to_string())
}

/// One breaking wave, or one lap against the hull: noise through a low-pass
/// whose cutoff falls from `start_hz` to `end_hz` as it dies, so it starts as
/// a crash and settles into a hiss.
#[derive(Clone, Copy, Debug)]
struct Burst {
    age: f32,
    attack: f32,
    decay: f32,
    gain: f32,
    start_hz: f32,
    end_hz: f32,
    pan: [f32; 2],
    low_pass: [f32; 2],
    high_pass: f32,
    previous: f32,
}

impl Burst {
    fn envelope(&self) -> f32 {
        if self.age < self.attack {
            let t = self.age / self.attack;
            t * t
        } else {
            (-(self.age - self.attack) / self.decay).exp()
        }
    }

    fn finished(&self) -> bool {
        self.age > self.attack + 6.0 * self.decay
    }

    fn next(&mut self, white: f32, sample_rate: f32) -> [f32; 2] {
        let cutoff = self.end_hz
            + (self.start_hz - self.end_hz) * (-self.age / (0.4 * self.decay + self.attack)).exp();
        let a = one_pole(cutoff, sample_rate);
        self.low_pass[0] += a * (white - self.low_pass[0]);
        self.low_pass[1] += a * (self.low_pass[0] - self.low_pass[1]);
        // Take out the rumble below ~150 Hz: the roar layer owns that.
        let filtered = self.low_pass[1];
        self.high_pass = 0.98 * (self.high_pass + filtered - self.previous);
        self.previous = filtered;
        let value = self.high_pass * self.envelope() * self.gain;
        self.age += 1.0 / sample_rate;
        [value * self.pan[0], value * self.pan[1]]
    }
}

fn one_pole(cutoff_hz: f32, sample_rate: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * cutoff_hz / sample_rate).exp()
}

/// The synthesiser. Pure and deterministic for a seed, so it can be tested
/// without an audio device.
pub(crate) struct SeaSynth {
    sample_rate: f32,
    rng: u32,
    targets: [f32; 3],
    roughness: f32,
    level: f32,
    muffle: f32,
    ease: f32,
    brown: [f32; 2],
    roar: [[f32; 2]; 2],
    wobble: f32,
    breaks: Vec<Burst>,
    laps: Vec<Burst>,
    muffled: [[f32; 2]; 2],
}

impl SeaSynth {
    pub(crate) fn new(sample_rate: f32, seed: u32) -> Self {
        Self {
            sample_rate,
            rng: seed.max(1),
            targets: [0.0; 3],
            roughness: 0.0,
            level: 0.0,
            muffle: 0.0,
            ease: 1.0 - (-1.0 / (PARAMETER_EASE_SECONDS * sample_rate)).exp(),
            brown: [0.0; 2],
            roar: [[0.0; 2]; 2],
            wobble: 0.0,
            breaks: Vec::with_capacity(MAX_BREAKS),
            laps: Vec::with_capacity(MAX_LAPS),
            muffled: [[0.0; 2]; 2],
        }
    }

    /// Starts at these values without easing in (for tests and the demo).
    #[cfg(test)]
    pub(crate) fn jump_to(&mut self, roughness: f32, level: f32, muffle: f32) {
        self.set_targets(roughness, level, muffle);
        (self.roughness, self.level, self.muffle) = (roughness, level, muffle);
    }

    pub(crate) fn set_targets(&mut self, roughness: f32, level: f32, muffle: f32) {
        self.targets = [roughness, level, muffle];
    }

    fn random(&mut self) -> f32 {
        // xorshift32: enough for noise, and the same on every run.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / 16_777_216.0
    }

    fn white(&mut self) -> f32 {
        2.0 * self.random() - 1.0
    }

    fn pan(&mut self) -> [f32; 2] {
        let angle = (0.15 + 0.7 * self.random()) * std::f32::consts::FRAC_PI_2;
        [angle.cos(), angle.sin()]
    }

    fn spawn(&mut self) {
        let r = self.roughness;
        let dt = 1.0 / self.sample_rate;
        let break_rate = BREAK_RATE_CALM + (BREAK_RATE_STORM - BREAK_RATE_CALM) * r;
        if self.breaks.len() < MAX_BREAKS && self.random() < break_rate * dt {
            let attack = (0.2 + 0.5 * self.random()) * (1.0 + 0.5 * r);
            let decay = (0.9 + 2.2 * self.random()) * (1.0 + 1.2 * r);
            let gain = (0.35 + 0.45 * self.random()) * (0.45 + 0.55 * r);
            let start_hz = 1800.0 + 1600.0 * self.random();
            let end_hz = (380.0 + 380.0 * self.random()) * (1.0 - 0.35 * r);
            let pan = self.pan();
            self.breaks.push(Burst {
                age: 0.0,
                attack,
                decay,
                gain,
                start_hz,
                end_hz,
                pan,
                low_pass: [0.0; 2],
                high_pass: 0.0,
                previous: 0.0,
            });
        }
        let lap_rate = LAP_RATE_CALM * (1.0 - r) * (1.0 - r);
        if self.laps.len() < MAX_LAPS && self.random() < lap_rate * dt {
            let attack = 0.015 + 0.03 * self.random();
            let decay = 0.06 + 0.18 * self.random();
            let gain = (0.5 + 0.5 * self.random()) * 0.55 * (1.0 - r);
            let start_hz = 2500.0 + 2000.0 * self.random();
            let end_hz = 900.0 + 700.0 * self.random();
            let pan = self.pan();
            self.laps.push(Burst {
                age: 0.0,
                attack,
                decay,
                gain,
                start_hz,
                end_hz,
                pan,
                low_pass: [0.0; 2],
                high_pass: 0.0,
                previous: 0.0,
            });
        }
    }

    pub(crate) fn next_frame(&mut self) -> [f32; 2] {
        self.roughness += (self.targets[0] - self.roughness) * self.ease;
        self.level += (self.targets[1] - self.level) * self.ease;
        self.muffle += (self.targets[2] - self.muffle) * self.ease;
        self.spawn();
        let r = self.roughness;
        let sample_rate = self.sample_rate;
        let mut out = [0.0_f32; 2];

        // The roar: brown noise, separate per ear for width, low-passed lower
        // and louder as the sea builds, with a slow swell in it.
        self.wobble = (self.wobble + 0.13 / sample_rate) % 1.0;
        let swell = 1.0 + 0.12 * (std::f32::consts::TAU * self.wobble).sin();
        let roar_gain = (ROAR_FLOOR + (ROAR_GAIN - ROAR_FLOOR) * r.powf(1.3)) * swell;
        let roar_a = one_pole(260.0 + 360.0 * r, sample_rate);
        for ear in 0..2 {
            let white = self.white();
            self.brown[ear] = 0.998 * self.brown[ear] + 0.03 * white;
            self.roar[ear][0] += roar_a * (self.brown[ear] - self.roar[ear][0]);
            self.roar[ear][1] += roar_a * (self.roar[ear][0] - self.roar[ear][1]);
            out[ear] += self.roar[ear][1] * roar_gain;
        }

        for index in 0..self.breaks.len() {
            let white = self.white();
            let [left, right] = self.breaks[index].next(white, sample_rate);
            out[0] += left;
            out[1] += right;
        }
        self.breaks.retain(|burst| !burst.finished());
        for index in 0..self.laps.len() {
            let white = self.white();
            let [left, right] = self.laps[index].next(white, sample_rate);
            out[0] += left;
            out[1] += right;
        }
        self.laps.retain(|burst| !burst.finished());

        // Under the water: everything dull and low.
        let muffle_a = one_pole(16_000.0 + (320.0 - 16_000.0) * self.muffle, sample_rate);
        let muffle_gain = 1.0 - 0.3 * self.muffle;
        for ear in 0..2 {
            self.muffled[ear][0] += muffle_a * (out[ear] - self.muffled[ear][0]);
            self.muffled[ear][1] += muffle_a * (self.muffled[ear][0] - self.muffled[ear][1]);
            out[ear] = soft_clip(self.muffled[ear][1] * muffle_gain * self.level);
        }
        out
    }
}

/// Rounds off peaks rather than clipping them.
fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// Loudness (RMS) in 100ms windows over `seconds`.
    fn window_loudness(roughness: f32, seconds: f32, seed: u32) -> Vec<f32> {
        let mut synth = SeaSynth::new(RATE, seed);
        synth.jump_to(roughness, 1.0, 0.0);
        // Let the sea get going before listening.
        for _ in 0..(RATE as usize * 4) {
            synth.next_frame();
        }
        let window = (RATE * 0.1) as usize;
        (0..(seconds * 10.0) as usize)
            .map(|_| {
                let sum: f32 = (0..window)
                    .map(|_| {
                        let [l, r] = synth.next_frame();
                        0.5 * (l * l + r * r)
                    })
                    .sum();
                (sum / window as f32).sqrt()
            })
            .collect()
    }

    fn mean_and_variation(values: &[f32]) -> (f32, f32) {
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        let variance =
            values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32;
        (mean, variance.sqrt() / mean)
    }

    #[test]
    fn a_rougher_sea_is_louder_and_steadier_until_it_is_a_roar() {
        let (calm, calm_variation) = mean_and_variation(&window_loudness(0.0, 30.0, 7));
        let (moderate, moderate_variation) = mean_and_variation(&window_loudness(0.5, 30.0, 7));
        let (storm, storm_variation) = mean_and_variation(&window_loudness(1.0, 30.0, 7));
        // Louder as the waves grow.
        assert!(
            calm < moderate && moderate < storm,
            "{calm} {moderate} {storm}"
        );
        // A slight sea is separate breaks with quiet between; a storm's
        // overlap into a nearly steady roar.
        assert!(
            calm_variation > moderate_variation && moderate_variation > storm_variation,
            "{calm_variation} {moderate_variation} {storm_variation}"
        );
        assert!(
            storm_variation < 0.25,
            "storm loudness varies by {storm_variation}"
        );
        assert!(calm_variation > 2.0 * storm_variation);
    }

    #[test]
    fn the_sea_never_clips_and_is_silent_away_from_the_water() {
        let mut synth = SeaSynth::new(RATE, 3);
        synth.jump_to(1.0, 1.0, 0.0);
        let peak = (0..(RATE as usize * 10))
            .map(|_| {
                let [l, r] = synth.next_frame();
                l.abs().max(r.abs())
            })
            .fold(0.0_f32, f32::max);
        assert!(peak < 1.0, "peak {peak}");
        let mut away = SeaSynth::new(RATE, 3);
        away.jump_to(1.0, 0.0, 0.0);
        assert!((0..4800).all(|_| away.next_frame() == [0.0, 0.0]));
    }

    #[test]
    fn under_the_water_the_sea_is_muffled() {
        // Share of the loudness above 2kHz, in air and under water.
        let brightness = |muffle: f32| {
            let mut synth = SeaSynth::new(RATE, 11);
            synth.jump_to(0.3, 1.0, muffle);
            let mut previous = 0.0_f32;
            let (mut total, mut high) = (0.0_f32, 0.0_f32);
            for _ in 0..(RATE as usize * 10) {
                let [l, _] = synth.next_frame();
                total += l * l;
                // A first difference weights high frequencies.
                high += (l - previous) * (l - previous);
                previous = l;
            }
            high / total.max(1.0e-12)
        };
        assert!(brightness(1.0) < 0.2 * brightness(0.0));
    }

    #[test]
    fn the_sea_is_quieter_higher_above_it() {
        assert!((loudness_at_height(0.0) - 1.0).abs() < 1.0e-6);
        assert!((loudness_at_height(HALF_LOUDNESS_HEIGHT_METERS) - 0.5).abs() < 1.0e-6);
        assert!(loudness_at_height(2_000.0) < 0.05);
        assert_eq!(loudness_at_height(-3.0), 1.0);
    }

    /// Writes a 30-second calm-to-storm sample to listen to:
    /// `cargo test --release -p catinthegarden-app write_sea_sound_demo -- --ignored`.
    #[test]
    #[ignore = "writes a WAV to listen to"]
    fn write_sea_sound_demo() {
        let rate = 44_100_u32;
        let seconds = 30.0_f32;
        let mut synth = SeaSynth::new(rate as f32, 5);
        synth.jump_to(0.0, 1.0, 0.0);
        let frames = (rate as f32 * seconds) as usize;
        let mut pcm = Vec::with_capacity(frames * 4);
        for frame in 0..frames {
            let t = frame as f32 / (rate as f32 * seconds);
            // Hold calm, build over the middle, hold the storm.
            let r = ((t - 0.15) / 0.6).clamp(0.0, 1.0);
            synth.set_targets(r * r * (3.0 - 2.0 * r), 1.0, 0.0);
            for value in synth.next_frame() {
                pcm.extend_from_slice(&((value * 32_767.0) as i16).to_le_bytes());
            }
        }
        let mut wav = Vec::new();
        let data = pcm.len() as u32;
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 4).to_le_bytes());
        wav.extend_from_slice(&4_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data.to_le_bytes());
        wav.extend_from_slice(&pcm);
        let path = std::env::temp_dir().join("sea_sound_calm_to_storm.wav");
        std::fs::write(&path, wav).expect("write demo");
        println!("wrote {}", path.display());
    }
}
