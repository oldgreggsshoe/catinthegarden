//! The sound of the sea and the wind, synthesised. The one recording is the
//! thunder (`thunder_clips`): a CC0 clip per strike, falling back to a
//! synthesised low thump if the clips are missing.
//!
//! Three layers of filtered noise. Breaking waves are bursts that start bright
//! (the crash) and sweep down into a hiss (the wash), arriving at random and
//! more often as the sea gets up. Small splashes lap against the hull in a
//! slight sea. Under both, a low steady roar. In a small sea the breaks are
//! separate, with quiet between; as the waves grow they come faster, longer
//! and louder and the roar rises, until in a storm they overlap into one
//! nearly stable roar.
//!
//! The wind is separate, and heard wherever there is air, however high: a
//! rushing roar that rises with its speed, resonant howls that drift in pitch
//! and swell in and out once it blows hard, and a thin whistle in the
//! strongest gusts.
//!
//! The game sets roughness, listener level, submersion, wind and game-clock
//! rate each frame (`SeaSound::set`). The audio thread eases the first four;
//! the clock rate controls break frequency and duration, and zero silences it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::thunder_clips::{ThunderClips, thunder_cutoff_hz};

/// Overall loudness; `PLANET_SOUND_VOLUME` scales it (0-2).
const VOLUME: f32 = 0.4;
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
const MAX_CREAKS: usize = 4;
const MAX_IMPACTS: usize = 6;
/// Thunder clips playing at once; a new strike past this replaces the oldest.
const MAX_THUNDER_VOICES: usize = 4;
/// Level of a recorded thunder clip (loudness-matched by `thunder_clips`)
/// before the strike's own gain. Set against a full storm's sea and wind on
/// deck: a close strike stands well over them (peaks rounded by the soft
/// clip), a far roll a little over them.
const THUNDER_CLIP_GAIN: f32 = 5.0;
/// Metres of water over the listener at which the surface sounds (the sea's
/// roar and breaking waves, the wind, thunder, the hull's creaks above the
/// waves) have fallen to 1/e; a few times this and they are gone, and the
/// deep takes over.
const SURFACE_SOUND_DEPTH_EFOLD_METERS: f32 = 5.0;
/// Gain of the deep-water rumble heard once the surface sounds are gone.
const DEEP_RUMBLE_GAIN: f32 = 0.1;
/// Loudness of the low roar at full storm; in a calm it is a faint floor.
const ROAR_GAIN: f32 = 1.1;
const ROAR_FLOOR: f32 = 0.06;
/// Wind: the roar at full wind, the howls (from WIND_HOWL_ONSET), the whistle
/// (from WIND_WHISTLE_ONSET), as shares of full wind (`SeaSound::set`).
const WIND_ROAR_GAIN: f32 = 0.9;
const WIND_HOWL_GAIN: f32 = 0.45;
const WIND_WHISTLE_GAIN: f32 = 0.08;
const WIND_HOWL_ONSET: f32 = 0.35;
const WIND_WHISTLE_ONSET: f32 = 0.6;
/// The howls' resonances at full wind (Hz), and how sharp they are.
const WIND_HOWL_PITCHES: [f32; 3] = [330.0, 480.0, 700.0];
const WIND_HOWL_Q: f32 = 12.0;
const WIND_WHISTLE_Q: f32 = 30.0;
/// Distance above the water (m) at which the sea is half as loud.
pub const HALF_LOUDNESS_HEIGHT_METERS: f64 = 20.0;

/// Loudness of the sea for a listener this far above the water (m): full at
/// the surface, half at `HALF_LOUDNESS_HEIGHT_METERS`, faint from the air.
pub fn loudness_at_height(height_meters: f64) -> f32 {
    (1.0 / (1.0 + height_meters.max(0.0) / HALF_LOUDNESS_HEIGHT_METERS)) as f32
}

fn volume() -> f32 {
    std::env::var("PLANET_SOUND_VOLUME")
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map_or(1.0, |value| value.clamp(0.0, 2.0))
        * VOLUME
}

/// `PLANET_SOUND_SEA=0` mutes the sea (roar, breaks, laps) and leaves the
/// wind, thunder and the rest, to hear them on their own.
fn sea_enabled() -> bool {
    !matches!(
        std::env::var("PLANET_SOUND_SEA")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("0" | "off" | "false")
    )
}

/// `PLANET_SOUND=0` keeps the game silent.
fn enabled() -> bool {
    !matches!(
        std::env::var("PLANET_SOUND")
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
    wind: AtomicU32,
    clock_rate: AtomicU32,
    thunder_gain: AtomicU32,
    thunder_pan: AtomicU32,
    thunder_distance: AtomicU32,
    thunder_sequence: AtomicU32,
    /// Recorded thunder, decoded off the audio thread after the stream opens.
    thunder_clips: std::sync::OnceLock<Arc<ThunderClips>>,
    creak_stress: AtomicU32,
    depth: AtomicU32,
    scrape: AtomicU32,
    tumble: AtomicU32,
    impact_gain: AtomicU32,
    impact_sequence: AtomicU32,
}

fn store(slot: &AtomicU32, value: f32) {
    slot.store(value.to_bits(), Ordering::Relaxed);
}

fn load(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.load(Ordering::Relaxed))
}

pub struct SeaSound {
    sea_on: bool,
    shared: Arc<Shared>,
    _stream: Option<cpal::Stream>,
}

impl SeaSound {
    /// No output at all: replays and tests.
    pub fn silent() -> Self {
        Self {
            sea_on: true,
            shared: Arc::new(Shared::default()),
            _stream: None,
        }
    }

    /// Opens the default output device. Without one (or with
    /// `PLANET_SOUND=0`) the game carries on silently.
    pub fn start() -> Self {
        let shared = Arc::new(Shared::default());
        let stream = if enabled() {
            match open_stream(Arc::clone(&shared)) {
                Ok(stream) => {
                    let loading = Arc::clone(&shared);
                    std::thread::Builder::new()
                        .name("thunder-clips".into())
                        .spawn(move || match ThunderClips::load() {
                            Some(clips) => {
                                tracing::info!(
                                    target: "planet::sound",
                                    clips = clips.clips.len(),
                                    "thunder clips loaded"
                                );
                                let _ = loading.thunder_clips.set(Arc::new(clips));
                            }
                            None => tracing::warn!(
                                target: "planet::sound",
                                "no thunder clips; using the synthesised thump"
                            ),
                        })
                        .ok();
                    Some(stream)
                }
                Err(error) => {
                    tracing::warn!(target: "planet::sound", %error, "no sea sound");
                    None
                }
            }
        } else {
            None
        };
        Self {
            sea_on: sea_enabled(),
            shared,
            _stream: stream,
        }
    }

    /// `roughness` 0 (slight sea) to 1 (storm); `level` 0-1 how near the water
    /// the listener is (`loudness_at_height`, 0 away from the sea); `muffle`
    /// 0 in air, 1 under the water; `wind` 0 (still air) to 1 (a full gale
    /// past the listener); `clock_rate` is game time per real second, or zero
    /// while the scene is frozen.
    pub fn set(
        &self,
        roughness: f32,
        level: f32,
        muffle: f32,
        wind: f32,
        clock_rate: f32,
        creak_stress: f32,
    ) {
        store(&self.shared.roughness, roughness.clamp(0.0, 1.0));
        store(
            &self.shared.level,
            if self.sea_on { level.clamp(0.0, 1.0) } else { 0.0 },
        );
        store(&self.shared.muffle, muffle.clamp(0.0, 1.0));
        store(&self.shared.wind, wind.clamp(0.0, 1.0));
        store(&self.shared.clock_rate, clock_rate.max(0.0));
        store(&self.shared.creak_stress, creak_stress.clamp(0.0, 1.0));
    }

    /// What is under the water. `depth` is how far below the surface the
    /// listener is (m): the surface sounds fade out with it and the deep
    /// rumble fades in. `scrape` and `tumble` (0-1) are a hull sliding and
    /// rolling along the seabed near the listener.
    pub fn set_underwater(&self, depth: f32, scrape: f32, tumble: f32) {
        store(&self.shared.depth, depth.max(0.0));
        store(&self.shared.scrape, scrape.clamp(0.0, 1.0));
        store(&self.shared.tumble, tumble.clamp(0.0, 1.0));
    }

    /// A hull striking the seabed, `gain` 0-1 for how hard.
    pub fn seabed_impact(&self, gain: f32) {
        store(&self.shared.impact_gain, gain.clamp(0.0, 1.0));
        self.shared.impact_sequence.fetch_add(1, Ordering::Release);
    }

    /// One delayed thunder arrival from a strike `distance_meters` away. The
    /// sequence is published last so the audio callback sees the gain, stereo
    /// direction and distance together.
    pub fn thunder(&self, gain: f32, pan: f32, distance_meters: f32) {
        store(&self.shared.thunder_gain, gain.clamp(0.0, 1.0));
        store(&self.shared.thunder_pan, pan.clamp(0.0, 1.0));
        store(&self.shared.thunder_distance, distance_meters.max(0.0));
        self.shared.thunder_sequence.fetch_add(1, Ordering::Release);
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
    tracing::info!(target: "planet::sound", "sea sound playing");
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
    synth.volume = volume();
    let mut last_thunder_sequence = 0;
    let mut last_impact_sequence = 0;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                synth.set_targets(
                    load(&shared.roughness),
                    load(&shared.level),
                    load(&shared.muffle),
                    load(&shared.wind),
                );
                synth.clock_rate = load(&shared.clock_rate);
                synth.creak_target = load(&shared.creak_stress);
                synth.set_underwater(
                    load(&shared.depth),
                    load(&shared.scrape),
                    load(&shared.tumble),
                );
                let impacts = shared.impact_sequence.load(Ordering::Acquire);
                if impacts != last_impact_sequence {
                    synth.trigger_impact(load(&shared.impact_gain));
                    last_impact_sequence = impacts;
                }
                if synth.thunder_clips.is_none() {
                    synth.thunder_clips = shared.thunder_clips.get().cloned();
                }
                let sequence = shared.thunder_sequence.load(Ordering::Acquire);
                if sequence != last_thunder_sequence {
                    synth.trigger_thunder(
                        load(&shared.thunder_gain),
                        load(&shared.thunder_pan),
                        load(&shared.thunder_distance),
                    );
                    last_thunder_sequence = sequence;
                }
                let gains: Vec<[f32; 2]> = (0..channels)
                    .map(|channel| channel_gains(channels, channel))
                    .collect();
                for frame in data.chunks_mut(channels) {
                    let [left, right] = synth.next_frame();
                    for (sample, [from_left, from_right]) in frame.iter_mut().zip(&gains) {
                        *sample = T::from_sample(left * from_left + right * from_right);
                    }
                }
            },
            |error| tracing::warn!(target: "planet::sound", %error, "sea sound stream"),
            None,
        )
        .map_err(|error| error.to_string())
}

/// Share of the synthesised left and right that goes to output channel
/// `channel` of `channels`. The front pair takes them; in the standard quad,
/// 5.1 and 7.1 orders (front L/R, then centre and LFE, then back and side
/// pairs) the surround pairs take a softer copy so the sea still surrounds the
/// listener; the centre and the subwoofer take nothing (a sea of noise in the
/// LFE is a boom). Mono takes both sides mixed.
fn channel_gains(channels: usize, channel: usize) -> [f32; 2] {
    const SURROUND: f32 = 0.7;
    match (channels, channel) {
        (1, _) => [0.5, 0.5],
        (_, 0) => [1.0, 0.0],
        (_, 1) => [0.0, 1.0],
        (4, 2) | (6, 4) | (8, 4) | (8, 6) => [SURROUND, 0.0],
        (4, 3) | (6, 5) | (8, 5) | (8, 7) => [0.0, SURROUND],
        _ => [0.0, 0.0],
    }
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

    fn next(&mut self, white: f32, sample_rate: f32, clock_rate: f32) -> [f32; 2] {
        let cutoff = self.end_hz
            + (self.start_hz - self.end_hz) * (-self.age / (0.4 * self.decay + self.attack)).exp();
        let a = one_pole(cutoff, sample_rate);
        self.low_pass[0] += a * (white - self.low_pass[0]);
        self.low_pass[1] += a * (self.low_pass[0] - self.low_pass[1]);
        // Keep the body of a crashing wave. The former ~150 Hz high-pass and
        // 2.4-4.6 kHz onset made each break a thin, hard metallic clatter.
        let filtered = self.low_pass[1];
        self.high_pass = 0.995 * (self.high_pass + filtered - self.previous);
        self.previous = filtered;
        let value = self.high_pass * self.envelope() * self.gain;
        self.age += clock_rate / sample_rate;
        [value * self.pan[0], value * self.pan[1]]
    }
}

/// A low, sliding wood groan with a little friction noise. No narrow bandpass
/// resonance: that turns hull movement into another metallic ringing sound.
struct Creak {
    age: f32,
    duration: f32,
    pitch: f32,
    glide: f32,
    phase: f32,
    gain: f32,
    pan: [f32; 2],
    friction: f32,
}

impl Creak {
    fn next(&mut self, white: f32, sample_rate: f32, clock_rate: f32) -> [f32; 2] {
        let t = (self.age / self.duration).min(1.0);
        self.phase += std::f32::consts::TAU * self.pitch * (1.0 + self.glide * t) / sample_rate;
        self.friction += one_pole(600.0, sample_rate) * (white - self.friction);
        let envelope = smoothstep(0.0, 0.15, self.age)
            * (1.0 - smoothstep(self.duration - 0.3, self.duration, self.age));
        let groan = self.phase.sin() + 0.28 * (2.0 * self.phase).sin();
        let value = (0.75 * groan + 0.25 * self.friction) * envelope * self.gain;
        self.age += clock_rate / sample_rate;
        [value * self.pan[0], value * self.pan[1]]
    }
}

/// A hull striking the seabed: a low thump that sags in pitch, a duller
/// crunch of timber and sediment, and a brief knock of wood.
struct Impact {
    age: f32,
    gain: f32,
    phase: f32,
    crunch: f32,
    pan: [f32; 2],
}

impl Impact {
    const DURATION: f32 = 2.5;

    fn next(&mut self, white: f32, sample_rate: f32, clock_rate: f32) -> [f32; 2] {
        let pitch = 28.0 + 40.0 * (-self.age / 0.2).exp();
        self.phase += std::f32::consts::TAU * pitch / sample_rate;
        let thump = self.phase.sin() * (-self.age / 0.3).exp();
        self.crunch += one_pole(240.0, sample_rate) * (white - self.crunch);
        let crunch = self.crunch * (-self.age / 0.1).exp();
        let knock = white * (-self.age / 0.012).exp() * 0.3;
        let value = (1.1 * thump + 1.4 * crunch + knock)
            * self.gain
            * smoothstep(0.0, 0.004, self.age);
        self.age += clock_rate / sample_rate;
        [value * self.pan[0], value * self.pan[1]]
    }
}

fn one_pole(cutoff_hz: f32, sample_rate: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * cutoff_hz / sample_rate).exp()
}

fn xorshift(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as f32 / 16_777_216.0
}

/// A resonant band-pass (Chamberlin state-variable filter): the howls and the
/// whistle are white noise rung through one.
#[derive(Clone, Copy, Debug, Default)]
struct Resonator {
    low: f32,
    band: f32,
}

impl Resonator {
    fn next(&mut self, input: f32, centre_hz: f32, q: f32, sample_rate: f32) -> f32 {
        let f = 2.0 * (std::f32::consts::PI * centre_hz.min(sample_rate / 7.0) / sample_rate).sin();
        let high = input - self.low - self.band / q;
        self.band += f * high;
        self.low += f * self.band;
        // Scaled so a sharper resonance does not ring louder.
        self.band / q.sqrt()
    }
}

/// A value wandering smoothly between random points in -1..1, a new one every
/// `period` seconds: the wind's gusting, and the howls' drift and swell.
#[derive(Clone, Copy, Debug)]
struct Wander {
    from: f32,
    to: f32,
    t: f32,
    period: f32,
}

impl Wander {
    fn new(period: f32) -> Self {
        Self {
            from: 0.0,
            to: 0.0,
            t: 0.0,
            period,
        }
    }

    fn next(&mut self, dt: f32, rng: &mut u32) -> f32 {
        self.t += dt / self.period;
        if self.t >= 1.0 {
            self.t -= 1.0;
            self.from = self.to;
            self.to = 2.0 * xorshift(rng) - 1.0;
        }
        let s = self.t * self.t * (3.0 - 2.0 * self.t);
        self.from + (self.to - self.from) * s
    }
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The wind: a roar of pink noise per ear, three howls and a whistle.
struct Wind {
    pink: [[f32; 3]; 2],
    roar_low_pass: [[f32; 2]; 2],
    roar_high_pass: [[f32; 2]; 2],
    gusting: Wander,
    howls: [Resonator; 3],
    howl_pitch: [Wander; 3],
    howl_swell: [Wander; 3],
    whistle: Resonator,
    whistle_pitch: Wander,
    whistle_swell: Wander,
}

impl Wind {
    fn new() -> Self {
        Self {
            pink: [[0.0; 3]; 2],
            roar_low_pass: [[0.0; 2]; 2],
            roar_high_pass: [[0.0; 2]; 2],
            gusting: Wander::new(0.7),
            howls: [Resonator::default(); 3],
            howl_pitch: [Wander::new(2.3), Wander::new(3.1), Wander::new(1.9)],
            howl_swell: [Wander::new(1.7), Wander::new(2.6), Wander::new(2.1)],
            whistle: Resonator::default(),
            whistle_pitch: Wander::new(1.3),
            whistle_swell: Wander::new(0.9),
        }
    }

    /// `strength` 0-1: how hard the air moves past the listener.
    fn next(&mut self, strength: f32, rng: &mut u32, sample_rate: f32) -> [f32; 2] {
        let dt = 1.0 / sample_rate;
        let gust = self.gusting.next(dt, rng);
        let mut out = [0.0_f32; 2];
        // Roar: pink noise (Kellet's filter), 90Hz up to a cutoff that rises
        // with the wind, flickering with its turbulence.
        let roar_gain = WIND_ROAR_GAIN * strength * strength * (1.0 + 0.45 * strength * gust);
        let low_a = one_pole(350.0 + 1300.0 * strength, sample_rate);
        for ear in 0..2 {
            let white = 2.0 * xorshift(rng) - 1.0;
            let p = &mut self.pink[ear];
            p[0] = 0.99765 * p[0] + white * 0.099_046;
            p[1] = 0.963 * p[1] + white * 0.296_516_4;
            p[2] = 0.57 * p[2] + white * 1.052_691_3;
            let pink = 0.25 * (p[0] + p[1] + p[2] + white * 0.1848);
            let [low_1, low_2] = &mut self.roar_low_pass[ear];
            *low_1 += low_a * (pink - *low_1);
            *low_2 += low_a * (*low_1 - *low_2);
            let [high, previous] = &mut self.roar_high_pass[ear];
            *high = 0.988 * (*high + *low_2 - *previous);
            *previous = *low_2;
            out[ear] += *high * roar_gain;
        }
        // Howls: resonances rung by the wind, higher as it strengthens,
        // drifting in pitch and swelling in and out.
        let howling =
            WIND_HOWL_GAIN * smoothstep(WIND_HOWL_ONSET, 0.85, strength) * (0.7 + 0.3 * gust);
        if howling > 0.0 {
            for index in 0..3 {
                let white = 2.0 * xorshift(rng) - 1.0;
                let pitch = WIND_HOWL_PITCHES[index]
                    * (0.55 + 0.45 * strength)
                    * (1.0 + 0.12 * self.howl_pitch[index].next(dt, rng));
                let swell = 0.5 + 0.5 * self.howl_swell[index].next(dt, rng);
                let value = self.howls[index].next(white, pitch, WIND_HOWL_Q, sample_rate)
                    * howling
                    * swell
                    * swell;
                let pan = [0.25, 0.5, 0.75][index];
                out[0] += value * (1.0 - pan);
                out[1] += value * pan;
            }
        }
        // Whistle: thin and high, in the strongest gusts only.
        let whistling = WIND_WHISTLE_GAIN
            * smoothstep(WIND_WHISTLE_ONSET, 0.95, strength)
            * smoothstep(-0.2, 0.8, gust);
        if whistling > 0.0 {
            let white = 2.0 * xorshift(rng) - 1.0;
            let pitch =
                (1500.0 + 1100.0 * strength) * (1.0 + 0.08 * self.whistle_pitch.next(dt, rng));
            let swell = 0.5 + 0.5 * self.whistle_swell.next(dt, rng);
            let value =
                self.whistle.next(white, pitch, WIND_WHISTLE_Q, sample_rate) * whistling * swell;
            out[0] += value;
            out[1] += value;
        }
        out
    }
}

/// One recorded thunder playing: where in its clip (in clip samples, stepped
/// by the clip's rate over the output's), its level, pan (0 left, 1 right),
/// and a two-pole low-pass for the air between.
struct ThunderVoice {
    clip: usize,
    position: f64,
    step: f64,
    gain: f32,
    pan: f32,
    lowpass: f32,
    low: [f32; 2],
}

/// The synthesiser. Pure and deterministic for a seed, so it can be tested
/// without an audio device.
pub(crate) struct SeaSynth {
    sample_rate: f32,
    rng: u32,
    /// Overall loudness (`volume`).
    pub(crate) volume: f32,
    targets: [f32; 4],
    roughness: f32,
    level: f32,
    muffle: f32,
    wind_strength: f32,
    clock_rate: f32,
    wind: Wind,
    ease: f32,
    brown: [f32; 2],
    roar: [[f32; 2]; 2],
    wobble: f32,
    breaks: Vec<Burst>,
    laps: Vec<Burst>,
    muffled: [[f32; 2]; 2],
    creak_target: f32,
    creak_stress: f32,
    creaks: Vec<Creak>,
    thunder_age: f32,
    thunder_gain: f32,
    thunder_pan: f32,
    thunder_low: f32,
    /// Recorded thunder, once loaded, the clips playing, and the last one
    /// started (not repeated next).
    pub(crate) thunder_clips: Option<Arc<ThunderClips>>,
    thunder_voices: Vec<ThunderVoice>,
    last_thunder_clip: Option<usize>,
    /// How far under the water the listener is, and the hull on the seabed:
    /// sliding, rolling, and the thumps of its landings.
    depth: f32,
    scrape: f32,
    tumble: f32,
    underwater_targets: [f32; 3],
    impacts: Vec<Impact>,
    scrape_phase: f32,
    scrape_bands: [f32; 2],
    tumble_brown: f32,
    tumble_low: f32,
    tumble_phase: f32,
    tumble_wander: f32,
    deep_brown: f32,
    deep_low: [f32; 2],
}

impl SeaSynth {
    pub(crate) fn new(sample_rate: f32, seed: u32) -> Self {
        Self {
            sample_rate,
            rng: seed.max(1),
            volume: 1.0,
            targets: [0.0; 4],
            roughness: 0.0,
            level: 0.0,
            muffle: 0.0,
            wind_strength: 0.0,
            clock_rate: 1.0,
            wind: Wind::new(),
            ease: 1.0 - (-1.0 / (PARAMETER_EASE_SECONDS * sample_rate)).exp(),
            brown: [0.0; 2],
            roar: [[0.0; 2]; 2],
            wobble: 0.0,
            breaks: Vec::with_capacity(MAX_BREAKS),
            laps: Vec::with_capacity(MAX_LAPS),
            muffled: [[0.0; 2]; 2],
            creak_target: 0.0,
            creak_stress: 0.0,
            creaks: Vec::with_capacity(MAX_CREAKS),
            thunder_age: f32::INFINITY,
            thunder_gain: 0.0,
            thunder_pan: 0.5,
            thunder_low: 0.0,
            thunder_clips: None,
            thunder_voices: Vec::with_capacity(MAX_THUNDER_VOICES),
            last_thunder_clip: None,
            depth: 0.0,
            scrape: 0.0,
            tumble: 0.0,
            underwater_targets: [0.0; 3],
            impacts: Vec::with_capacity(MAX_IMPACTS),
            scrape_phase: 0.0,
            scrape_bands: [0.0; 2],
            tumble_brown: 0.0,
            tumble_low: 0.0,
            tumble_phase: 0.0,
            tumble_wander: 0.0,
            deep_brown: 0.0,
            deep_low: [0.0; 2],
        }
    }

    /// Starts at these values without easing in (for tests and the demo).
    #[cfg(test)]
    pub(crate) fn jump_to(&mut self, roughness: f32, level: f32, muffle: f32, wind: f32) {
        self.set_targets(roughness, level, muffle, wind);
        (self.roughness, self.level, self.muffle, self.wind_strength) =
            (roughness, level, muffle, wind);
    }

    pub(crate) fn set_targets(&mut self, roughness: f32, level: f32, muffle: f32, wind: f32) {
        self.targets = [roughness, level, muffle, wind];
    }

    pub(crate) fn set_underwater(&mut self, depth: f32, scrape: f32, tumble: f32) {
        self.underwater_targets = [depth.max(0.0), scrape.clamp(0.0, 1.0), tumble.clamp(0.0, 1.0)];
    }

    pub(crate) fn trigger_impact(&mut self, gain: f32) {
        if self.impacts.len() >= MAX_IMPACTS {
            self.impacts.remove(0);
        }
        let pan = self.pan();
        self.impacts.push(Impact {
            age: 0.0,
            gain: gain.clamp(0.0, 1.0) * 1.6,
            phase: 0.0,
            crunch: 0.0,
            pan,
        });
    }

    /// A thunder arrival: a recorded clip chosen for the distance (a crack
    /// close by, a low roll far off) and dulled by it, or without clips the
    /// synthesised thump.
    fn trigger_thunder(&mut self, gain: f32, pan: f32, distance_meters: f32) {
        let Some(clips) = self.thunder_clips.clone() else {
            self.thunder_age = 0.0;
            self.thunder_gain = gain;
            self.thunder_pan = pan;
            return;
        };
        let (jitter, pick) = (self.random(), self.random());
        let index = clips.choose(distance_meters, jitter, pick, self.last_thunder_clip);
        self.last_thunder_clip = Some(index);
        if self.thunder_voices.len() >= MAX_THUNDER_VOICES {
            self.thunder_voices.remove(0);
        }
        self.thunder_voices.push(ThunderVoice {
            clip: index,
            position: 0.0,
            step: f64::from(clips.clips[index].rate / self.sample_rate),
            gain: gain * THUNDER_CLIP_GAIN * clips.clips[index].level,
            pan,
            lowpass: one_pole(thunder_cutoff_hz(distance_meters), self.sample_rate),
            low: [0.0; 2],
        });
    }

    /// The thunder clips playing, mixed to left and right by their pans,
    /// advanced one output sample at real speed (thunder does not change
    /// pitch with the game clock), finished ones dropped.
    fn thunder_clip_frame(&mut self) -> [f32; 2] {
        let Some(clips) = self.thunder_clips.as_ref() else {
            return [0.0; 2];
        };
        let mut out = [0.0_f32; 2];
        for voice in &mut self.thunder_voices {
            let samples = &clips.clips[voice.clip].samples;
            let index = voice.position as usize;
            if index + 1 >= samples.len() {
                voice.position = f64::INFINITY;
                continue;
            }
            let fraction = (voice.position - index as f64) as f32;
            let value = samples[index] + (samples[index + 1] - samples[index]) * fraction;
            voice.low[0] += voice.lowpass * (value - voice.low[0]);
            voice.low[1] += voice.lowpass * (voice.low[0] - voice.low[1]);
            let heard = voice.low[1] * voice.gain;
            out[0] += heard * (1.5 - voice.pan);
            out[1] += heard * (0.5 + voice.pan);
            voice.position += voice.step;
        }
        self.thunder_voices.retain(|voice| voice.position.is_finite());
        out
    }

    /// xorshift32: enough for noise, and the same on every run.
    fn random(&mut self) -> f32 {
        xorshift(&mut self.rng)
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
        let break_rate =
            (BREAK_RATE_CALM + (BREAK_RATE_STORM - BREAK_RATE_CALM) * r) * self.clock_rate;
        if self.breaks.len() < MAX_BREAKS && self.random() < break_rate * dt {
            // A quick rise, so each break lands as a crash before its wash.
            let attack = (0.10 + 0.35 * self.random()) * (1.0 + 0.5 * r);
            let decay = (0.9 + 2.2 * self.random()) * (1.0 + 1.2 * r);
            let gain = (0.35 + 0.45 * self.random()) * (0.45 + 0.55 * r);
            let start_hz = 900.0 + 1500.0 * self.random();
            let end_hz = (250.0 + 300.0 * self.random()) * (1.0 - 0.25 * r);
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
        let lap_rate = LAP_RATE_CALM * (1.0 - r) * (1.0 - r) * self.clock_rate;
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
        if self.creak_stress > 0.0 && self.creaks.len() < MAX_CREAKS
            && self.random() < 0.8 * self.creak_stress * self.clock_rate * dt
        {
            let duration = 0.65 + 1.1 * self.random();
            let pitch = 70.0 + 65.0 * self.random();
            let glide = 0.15 + 0.35 * self.random();
            let gain = 0.12 + 0.16 * self.random();
            let pan = self.pan();
            self.creaks.push(Creak {
                age: 0.0,
                duration,
                pitch,
                glide,
                phase: 0.0,
                gain,
                pan,
                friction: 0.0,
            });
        }
    }

    pub(crate) fn next_frame(&mut self) -> [f32; 2] {
        // Freeze the acoustic scene along with the game clock. Do not age
        // active breaks while paused, so they resume rather than restart.
        if self.clock_rate == 0.0 {
            return [0.0; 2];
        }
        self.roughness += (self.targets[0] - self.roughness) * self.ease;
        self.level += (self.targets[1] - self.level) * self.ease;
        self.muffle += (self.targets[2] - self.muffle) * self.ease;
        self.wind_strength += (self.targets[3] - self.wind_strength) * self.ease;
        self.creak_stress += (self.creak_target - self.creak_stress) * self.ease;
        self.depth += (self.underwater_targets[0] - self.depth) * self.ease;
        self.scrape += (self.underwater_targets[1] - self.scrape) * self.ease;
        self.tumble += (self.underwater_targets[2] - self.tumble) * self.ease;
        self.spawn();
        let r = self.roughness;
        let sample_rate = self.sample_rate;
        let mut out = [0.0_f32; 2];

        // The roar: brown noise, separate per ear for width, low-passed lower
        // and louder as the sea builds, with a slow swell in it.
        self.wobble = (self.wobble + 0.13 * self.clock_rate / sample_rate) % 1.0;
        let swell = 1.0 + 0.12 * (std::f32::consts::TAU * self.wobble).sin();
        let roar_gain = (ROAR_FLOOR + (ROAR_GAIN - ROAR_FLOOR) * r.powf(1.3)) * swell;
        let roar_a = one_pole(260.0 + 360.0 * r, sample_rate);
        let mut roar = [0.0_f32; 2];
        for ear in 0..2 {
            let white = self.white();
            self.brown[ear] = 0.998 * self.brown[ear] + 0.03 * white;
            self.roar[ear][0] += roar_a * (self.brown[ear] - self.roar[ear][0]);
            self.roar[ear][1] += roar_a * (self.roar[ear][0] - self.roar[ear][1]);
            roar[ear] = self.roar[ear][1] * roar_gain;
        }

        for index in 0..self.breaks.len() {
            let white = self.white();
            let [left, right] = self.breaks[index].next(white, sample_rate, self.clock_rate);
            out[0] += left;
            out[1] += right;
        }
        self.breaks.retain(|burst| !burst.finished());
        for index in 0..self.laps.len() {
            let white = self.white();
            let [left, right] = self.laps[index].next(white, sample_rate, self.clock_rate);
            out[0] += left;
            out[1] += right;
        }
        self.laps.retain(|burst| !burst.finished());
        let mut creak = [0.0_f32; 2];
        for index in 0..self.creaks.len() {
            let white = self.white();
            let sound = self.creaks[index].next(white, sample_rate, self.clock_rate);
            creak[0] += sound[0];
            creak[1] += sound[1];
        }
        self.creaks.retain(|sound| sound.age < sound.duration);

        // The sea fades with height above it; the wind does not. Under the
        // water there is almost none.
        let wind = if self.wind_strength > 0.0 {
            self.wind
                .next(self.wind_strength, &mut self.rng, sample_rate)
        } else {
            [0.0; 2]
        };
        let wind_level = 1.0 - 0.95 * self.muffle;
        let thunder_clips = self.thunder_clip_frame();
        let thunder = if self.thunder_age < 5.0 {
            let noise = self.white();
            let a = one_pole(90.0, sample_rate);
            self.thunder_low += a * (noise - self.thunder_low);
            let envelope =
                (1.0 - (-self.thunder_age / 0.08).exp()) * (-self.thunder_age / 1.5).exp();
            self.thunder_age += self.clock_rate / sample_rate;
            self.thunder_low * envelope * self.thunder_gain * 3.0
        } else {
            0.0
        };
        // The surface sounds die away with depth: a listener under the waves
        // hears none of the sea's crashing, the wind, the thunder or the
        // hull's creaks above them -- only the deep.
        let surface = (-self.depth / SURFACE_SOUND_DEPTH_EFOLD_METERS).exp();
        let mut under = [0.0_f32; 2];
        // The deep: a low rumble that grows as the surface lets go.
        for ear in 0..2 {
            let white = self.white();
            self.deep_brown = 0.998 * self.deep_brown + 0.03 * white;
            self.deep_low[ear] += one_pole(90.0, sample_rate) * (self.deep_brown - self.deep_low[ear]);
            under[ear] += self.deep_low[ear] * DEEP_RUMBLE_GAIN * (1.0 - surface);
        }
        // The hull on the bottom. Thumps first.
        for index in 0..self.impacts.len() {
            let white = self.white();
            let sound = self.impacts[index].next(white, sample_rate, self.clock_rate);
            under[0] += sound[0];
            under[1] += sound[1];
        }
        self.impacts.retain(|impact| impact.age < Impact::DURATION);
        // Sliding: gritty stick-slip noise between about 120 and 700 Hz,
        // juddering faster the faster it slides.
        if self.scrape > 0.001 {
            let white = self.white();
            let jitter = 0.4 * (self.random() - 0.5);
            self.scrape_phase = (self.scrape_phase
                + (5.0 + 9.0 * self.scrape + jitter) * self.clock_rate / sample_rate)
                .rem_euclid(1.0);
            let stick = smoothstep(0.0, 0.15, self.scrape_phase)
                * (1.0 - smoothstep(0.6, 1.0, self.scrape_phase));
            let grit = white * white.abs();
            self.scrape_bands[0] += one_pole(700.0, sample_rate) * (grit - self.scrape_bands[0]);
            self.scrape_bands[1] += one_pole(120.0, sample_rate) * (grit - self.scrape_bands[1]);
            let band = self.scrape_bands[0] - self.scrape_bands[1];
            let value = band * (0.35 + 0.65 * stick) * self.scrape * 7.0;
            under[0] += value;
            under[1] += value;
        }
        // Rolling: a rumble with a slow wooden groan in it.
        if self.tumble > 0.001 {
            let white = self.white();
            self.tumble_brown = 0.999 * self.tumble_brown + 0.02 * white;
            self.tumble_low += one_pole(80.0, sample_rate) * (self.tumble_brown - self.tumble_low);
            self.tumble_wander = (self.tumble_wander + 0.4 * self.clock_rate / sample_rate) % 1.0;
            let wander = (std::f32::consts::TAU * self.tumble_wander).sin();
            self.tumble_phase = (self.tumble_phase
                + std::f32::consts::TAU * (55.0 + 25.0 * wander) / sample_rate)
                .rem_euclid(std::f32::consts::TAU);
            let groan = self.tumble_phase.sin() + 0.3 * (2.0 * self.tumble_phase).sin();
            let value = self.tumble
                * (0.7 * self.tumble_low + 0.1 * groan * (0.5 + 0.5 * wander));
            under[0] += value;
            under[1] += value;
        }
        // Under the water: everything dull and low.
        let muffle_a = one_pole(16_000.0 + (320.0 - 16_000.0) * self.muffle, sample_rate);
        let muffle_gain = 1.0 - 0.3 * self.muffle;
        for ear in 0..2 {
            let side = if ear == 0 {
                1.0 - self.thunder_pan
            } else {
                self.thunder_pan
            };
            let mixed = surface
                * (roar[ear] * self.level
                    + out[ear] * self.level * self.level
                    + wind[ear] * wind_level
                    + creak[ear] * (1.0 - 0.8 * self.muffle)
                    + (thunder * (0.5 + side) + thunder_clips[ear]) * (1.0 - 0.8 * self.muffle))
                + under[ear];
            self.muffled[ear][0] += muffle_a * (mixed - self.muffled[ear][0]);
            self.muffled[ear][1] += muffle_a * (self.muffled[ear][0] - self.muffled[ear][1]);
            out[ear] = soft_clip(self.muffled[ear][1] * muffle_gain * self.volume);
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
        synth.jump_to(roughness, 1.0, 0.0, 0.0);
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
        synth.jump_to(1.0, 1.0, 0.0, 1.0);
        let peak = (0..(RATE as usize * 10))
            .map(|_| {
                let [l, r] = synth.next_frame();
                l.abs().max(r.abs())
            })
            .fold(0.0_f32, f32::max);
        assert!(peak < 1.0, "peak {peak}");
        let mut away = SeaSynth::new(RATE, 3);
        away.jump_to(1.0, 0.0, 0.0, 0.0);
        assert!((0..4800).all(|_| away.next_frame() == [0.0, 0.0]));
    }

    #[test]
    fn wave_breaks_follow_game_speed_and_all_sound_stops_when_frozen() {
        let break_count = |speed: f32| {
            let mut synth = SeaSynth::new(RATE, 31);
            synth.jump_to(0.0, 1.0, 0.0, 0.0);
            synth.clock_rate = speed;
            let mut births = 0;
            let mut previous = 0;
            for _ in 0..(RATE as usize * 60) {
                synth.next_frame();
                if synth.breaks.len() > previous {
                    births += synth.breaks.len() - previous;
                }
                previous = synth.breaks.len();
            }
            births
        };
        let normal = break_count(1.0);
        let fast = break_count(4.0);
        assert!(fast > 3 * normal && fast < 5 * normal, "{normal} {fast}");

        let mut synth = SeaSynth::new(RATE, 31);
        synth.jump_to(1.0, 1.0, 0.0, 1.0);
        for _ in 0..RATE as usize {
            synth.next_frame();
        }
        let ages: Vec<_> = synth.breaks.iter().map(|burst| burst.age).collect();
        synth.clock_rate = 0.0;
        assert!((0..RATE as usize).all(|_| synth.next_frame() == [0.0, 0.0]));
        assert_eq!(
            ages,
            synth
                .breaks
                .iter()
                .map(|burst| burst.age)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn thunder_is_a_delayed_sound_independent_of_the_sea() {
        let mut synth = SeaSynth::new(RATE, 53);
        synth.jump_to(0.0, 0.0, 0.0, 0.0);
        assert!((0..1000).all(|_| synth.next_frame() == [0.0, 0.0]));
        synth.trigger_thunder(1.0, 0.75, 3_000.0);
        let peak = (0..RATE as usize)
            .map(|_| {
                let [left, right] = synth.next_frame();
                assert!(right.abs() <= 1.0 && left.abs() <= 1.0);
                right.abs().max(left.abs())
            })
            .fold(0.0_f32, f32::max);
        assert!(peak > 0.01, "thunder peak {peak}");
        synth.clock_rate = 0.0;
        let age = synth.thunder_age;
        assert!((0..1000).all(|_| synth.next_frame() == [0.0, 0.0]));
        assert_eq!(synth.thunder_age, age);
    }

    /// Recorded thunder plays when loaded, pauses with the game clock, and a
    /// far strike comes through the air duller than a near one.
    #[test]
    fn recorded_thunder_plays_and_far_strikes_are_duller() {
        let clips = Arc::new(ThunderClips::load().expect("clips are in the repository"));
        let crossings = |distance: f32| {
            let mut synth = SeaSynth::new(RATE, 7);
            synth.jump_to(0.0, 0.0, 0.0, 0.0);
            synth.thunder_clips = Some(Arc::clone(&clips));
            synth.trigger_thunder(1.0, 0.5, distance);
            let frames: Vec<f32> = (0..(RATE as usize * 4))
                .map(|_| synth.next_frame()[0])
                .collect();
            let peak = frames.iter().fold(0.0_f32, |peak, value| peak.max(value.abs()));
            assert!(peak > 0.02, "thunder at {distance} m peaks at {peak}");
            synth.clock_rate = 0.0;
            assert!((0..1000).all(|_| synth.next_frame() == [0.0, 0.0]));
            let rms = (frames.iter().map(|v| v * v).sum::<f32>() / frames.len() as f32).sqrt();
            let changes = frames
                .windows(2)
                .filter(|pair| (pair[0] > 0.0) != (pair[1] > 0.0))
                .count();
            (changes as f32 / frames.len() as f32, rms)
        };
        let (near, _) = crossings(400.0);
        let (far, _) = crossings(8_500.0);
        assert!(far < 0.7 * near, "far {far} near {near} zero-crossing rate");
    }

    /// RMS of `seconds` of output after letting the parameters settle.
    fn settled_rms(synth: &mut SeaSynth, seconds: f32) -> f32 {
        for _ in 0..(RATE as usize * 6) {
            synth.next_frame();
        }
        let frames = (RATE * seconds) as usize;
        let sum: f32 = (0..frames)
            .map(|_| {
                let [l, r] = synth.next_frame();
                0.5 * (l * l + r * r)
            })
            .sum();
        (sum / frames as f32).sqrt()
    }

    #[test]
    fn the_surface_sounds_stop_under_the_waves() {
        let storm = |depth: f32| {
            let mut synth = SeaSynth::new(RATE, 11);
            synth.jump_to(1.0, 1.0, if depth > 0.0 { 1.0 } else { 0.0 }, 1.0);
            synth.creak_target = 1.0;
            synth.set_underwater(depth, 0.0, 0.0);
            // The depth eases in; start from it.
            synth.depth = depth;
            settled_rms(&mut synth, 2.0)
        };
        let above = storm(0.0);
        let shallow = storm(3.0);
        let deep = storm(40.0);
        assert!(above > 0.02, "storm above the water {above}");
        assert!(shallow < above, "{shallow} {above}");
        // At depth the storm is gone: what is left is the quiet deep.
        assert!(deep < 0.2 * above, "deep {deep} vs above {above}");
    }

    #[test]
    fn a_hull_landing_on_the_seabed_thumps_low_and_dies_away() {
        let mut synth = SeaSynth::new(RATE, 5);
        synth.jump_to(0.0, 0.0, 1.0, 0.0);
        synth.depth = 40.0;
        synth.set_underwater(40.0, 0.0, 0.0);
        let quiet = settled_rms(&mut synth, 1.0);
        synth.trigger_impact(1.0);
        let burst: Vec<f32> = (0..(RATE * 0.4) as usize).map(|_| synth.next_frame()[0]).collect();
        let peak = burst.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.05 && peak < 1.0, "thump peak {peak}");
        // Low: few zero crossings per second (a 28-70 Hz thump, not a click).
        let crossings = burst.windows(2).filter(|w| w[0].signum() != w[1].signum()).count();
        assert!((crossings as f32 / 0.4) < 400.0, "{crossings} crossings in 0.4s");
        // And it dies away to the deep's quiet.
        let tail = settled_rms(&mut synth, 1.0);
        assert!(tail < 3.0 * quiet.max(0.005), "tail {tail} quiet {quiet}");
    }

    #[test]
    fn scraping_and_tumbling_on_the_seabed_are_heard_only_while_they_happen() {
        let level = |scrape: f32, tumble: f32| {
            let mut synth = SeaSynth::new(RATE, 9);
            synth.jump_to(0.0, 0.0, 1.0, 0.0);
            synth.depth = 40.0;
            synth.scrape = scrape;
            synth.tumble = tumble;
            synth.set_underwater(40.0, scrape, tumble);
            settled_rms(&mut synth, 2.0)
        };
        let still = level(0.0, 0.0);
        let sliding = level(1.0, 0.0);
        let rolling = level(0.0, 1.0);
        println!("seabed levels: still {still:.4} sliding {sliding:.4} rolling {rolling:.4}");
        assert!(sliding > 2.0 * still, "{sliding} vs {still}");
        assert!(rolling > 2.0 * still, "{rolling} vs {still}");
    }

    #[test]
    fn hull_creaks_rise_with_stress_even_when_the_sea_layer_is_silent() {
        let mut synth = SeaSynth::new(RATE, 71);
        synth.jump_to(0.0, 0.0, 0.0, 0.0);
        assert!((0..RATE as usize).all(|_| synth.next_frame() == [0.0, 0.0]));
        synth.creak_target = 1.0;
        let peak = (0..(RATE as usize * 12))
            .map(|_| {
                let [left, right] = synth.next_frame();
                left.abs().max(right.abs())
            })
            .fold(0.0_f32, f32::max);
        assert!(peak > 0.01 && peak < 1.0, "creak peak {peak}");
        synth.clock_rate = 0.0;
        let ages: Vec<_> = synth.creaks.iter().map(|creak| creak.age).collect();
        assert!((0..1000).all(|_| synth.next_frame() == [0.0, 0.0]));
        assert_eq!(
            ages,
            synth
                .creaks
                .iter()
                .map(|creak| creak.age)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn under_the_water_the_sea_is_muffled() {
        // Share of the loudness above 2kHz, in air and under water.
        let brightness = |muffle: f32| {
            let mut synth = SeaSynth::new(RATE, 11);
            synth.jump_to(0.3, 1.0, muffle, 0.0);
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

    fn loudness(roughness: f32, level: f32, muffle: f32, wind: f32) -> f32 {
        let mut synth = SeaSynth::new(RATE, 21);
        synth.jump_to(roughness, level, muffle, wind);
        for _ in 0..(RATE as usize * 2) {
            synth.next_frame();
        }
        let frames = RATE as usize * 10;
        let sum: f32 = (0..frames)
            .map(|_| {
                let [l, r] = synth.next_frame();
                0.5 * (l * l + r * r)
            })
            .sum();
        (sum / frames as f32).sqrt()
    }

    #[test]
    fn the_wind_is_heard_high_above_the_sea_and_rises_with_its_strength() {
        // Away from the water (level 0) the sea is silent but the wind is not.
        assert_eq!(loudness(1.0, 0.0, 0.0, 0.0), 0.0);
        let breeze = loudness(0.0, 0.0, 0.0, 0.3);
        let wind = loudness(0.0, 0.0, 0.0, 0.6);
        let gale = loudness(0.0, 0.0, 0.0, 1.0);
        assert!(
            breeze > 0.0 && breeze < wind && wind < gale,
            "{breeze} {wind} {gale}"
        );
        // And almost none of it reaches under the water.
        assert!(loudness(0.0, 0.0, 1.0, 1.0) < 0.1 * gale);
    }

    #[test]
    fn surround_outputs_keep_the_sea_off_the_centre_and_the_subwoofer() {
        // Stereo and mono as expected.
        assert_eq!(channel_gains(2, 0), [1.0, 0.0]);
        assert_eq!(channel_gains(2, 1), [0.0, 1.0]);
        assert_eq!(channel_gains(1, 0), [0.5, 0.5]);
        // 5.1 (FL FR FC LFE BL BR): fronts full, centre and LFE silent, backs
        // a softer copy of their own side.
        let five_one: Vec<_> = (0..6).map(|channel| channel_gains(6, channel)).collect();
        assert_eq!(five_one[2], [0.0, 0.0]);
        assert_eq!(five_one[3], [0.0, 0.0]);
        assert!(five_one[4][0] > 0.0 && five_one[4][1] == 0.0);
        assert!(five_one[5][1] > 0.0 && five_one[5][0] == 0.0);
        // 7.1 adds the side pair; quad has only a back pair.
        assert_eq!(channel_gains(8, 3), [0.0, 0.0]);
        assert!(channel_gains(8, 6)[0] > 0.0 && channel_gains(8, 7)[1] > 0.0);
        assert!(channel_gains(4, 2)[0] > 0.0 && channel_gains(4, 3)[1] > 0.0);
        // An unknown layout gets the front pair only.
        assert_eq!(channel_gains(3, 2), [0.0, 0.0]);
    }

    #[test]
    fn the_sea_is_quieter_higher_above_it() {
        assert!((loudness_at_height(0.0) - 1.0).abs() < 1.0e-6);
        assert!((loudness_at_height(HALF_LOUDNESS_HEIGHT_METERS) - 0.5).abs() < 1.0e-6);
        assert!(loudness_at_height(2_000.0) < 0.05);
        assert_eq!(loudness_at_height(-3.0), 1.0);
    }

    /// Loudness of each layer on its own, for balancing them:
    /// `cargo test --release -p planet-app sea_and_wind_loudness -- --ignored --nocapture`.
    #[test]
    #[ignore = "instrument: prints loudness in dBFS"]
    fn sea_and_wind_loudness() {
        let db = |value: f32| 20.0 * value.max(1.0e-9).log10();
        for roughness in [0.0, 0.5, 1.0] {
            println!(
                "sea {roughness:.1}: {:6.1} dBFS",
                db(loudness(roughness, 1.0, 0.0, 0.0))
            );
        }
        for wind in [0.2, 0.45, 0.7, 1.0] {
            println!(
                "wind {wind:.2}: {:6.1} dBFS",
                db(loudness(0.0, 0.0, 0.0, wind))
            );
        }
    }

    /// Writes a 30-second calm-to-storm sample to listen to:
    /// `cargo test --release -p planet-app write_sea_sound_demo -- --ignored`.
    #[test]
    #[ignore = "writes a WAV to listen to"]
    fn write_sea_sound_demo() {
        let rate = 44_100_u32;
        let seconds = 30.0_f32;
        let mut synth = SeaSynth::new(rate as f32, 5);
        synth.volume = VOLUME;
        synth.jump_to(0.0, 1.0, 0.0, 0.45);
        let frames = (rate as f32 * seconds) as usize;
        let mut pcm = Vec::with_capacity(frames * 4);
        for frame in 0..frames {
            let t = frame as f32 / (rate as f32 * seconds);
            // Hold calm, build over the middle, hold the storm.
            let r = ((t - 0.15) / 0.6).clamp(0.0, 1.0);
            let storm = r * r * (3.0 - 2.0 * r);
            // The wind builds with the storm, gusting every few seconds.
            let gust = 0.08 * (frame as f32 / rate as f32 * 0.9).sin();
            synth.set_targets(storm, 1.0, 0.0, 0.45 + 0.45 * storm + gust * storm);
            for value in synth.next_frame() {
                pcm.extend_from_slice(&((value * 32_767.0) as i16).to_le_bytes());
            }
        }
        write_wav("sea_sound_calm_to_storm.wav", rate, &pcm);
    }

    /// Four thunders in a full storm on deck, from strikes 0.4, 2.5, 5 and
    /// 8.5 km away (a crack, two peals, a far roll), with the game's distance
    /// gain and pan. `cargo test --release -p planet-app write_thunder_demo
    /// -- --ignored`.
    #[test]
    #[ignore = "writes a WAV to listen to"]
    fn write_thunder_demo() {
        let rate = 44_100_u32;
        let mut synth = SeaSynth::new(rate as f32, 11);
        synth.volume = VOLUME;
        synth.jump_to(1.0, 0.6, 0.0, 0.8);
        synth.thunder_clips = Some(Arc::new(ThunderClips::load().expect("clips")));
        let strikes = [(1.0, 400.0, 0.3), (10.0, 2_500.0, 0.7), (21.0, 5_000.0, 0.45), (33.0, 8_500.0, 0.2)];
        let seconds = 52.0_f32;
        let mut pcm = Vec::new();
        for frame in 0..(rate as f32 * seconds) as usize {
            for (at, distance, pan) in strikes {
                if frame == (at * rate as f32) as usize {
                    synth.trigger_thunder(1.0 / (1.0 + distance / 3_000.0), pan, distance);
                }
            }
            for value in synth.next_frame() {
                pcm.extend_from_slice(&((value * 32_767.0) as i16).to_le_bytes());
            }
        }
        write_wav("thunder_in_a_storm.wav", rate, &pcm);
    }

    fn write_wav(name: &str, rate: u32, pcm: &[u8]) {
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
        wav.extend_from_slice(pcm);
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, wav).expect("write demo");
        println!("wrote {}", path.display());
    }
}
