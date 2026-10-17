//! Wall-clock timing of the startup jobs, from process start to the first few
//! rendered frames. `mark("x")` closes a phase called "x" that began at the
//! previous mark; the table is logged (target `planet::startup_profile`) and
//! printed to stderr, slowest first, once frame `REPORT_AFTER_FRAMES` begins.

use std::sync::Mutex;
use std::time::Instant;

const REPORT_AFTER_FRAMES: u32 = 4;

struct Profile {
    last: Instant,
    started: Instant,
    phases: Vec<(String, f64)>,
    frames_started: u32,
    reported: bool,
}

static PROFILE: Mutex<Option<Profile>> = Mutex::new(None);

pub fn begin() {
    let now = Instant::now();
    let _ = std::fs::write("test-runs/startup_profile_progress.txt", "");
    *PROFILE.lock().unwrap() = Some(Profile {
        last: now,
        started: now,
        phases: Vec::new(),
        frames_started: 0,
        reported: false,
    });
}

/// Closes the phase `label`, which began at the previous mark.
pub fn mark(label: &str) {
    if let Some(profile) = PROFILE.lock().unwrap().as_mut() {
        if profile.reported {
            return;
        }
        let now = Instant::now();
        profile
            .phases
            .push((label.to_string(), (now - profile.last).as_secs_f64() * 1e3));
        profile.last = now;
        // Written as it goes (the app redirects stderr and may be killed).
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("test-runs/startup_profile_progress.txt")
        {
            let (label, ms) = profile.phases.last().unwrap();
            let _ = writeln!(file, "{ms:9.1} ms  {label}");
        }
    }
}

/// Called at the start of every rendered frame; the first few close the
/// first-frame phases, and the table is reported when the last of them starts.
pub fn frame_started() {
    let n = {
        let mut guard = PROFILE.lock().unwrap();
        let Some(profile) = guard.as_mut() else {
            return;
        };
        if profile.reported {
            return;
        }
        profile.frames_started += 1;
        profile.frames_started
    };
    if n == 1 {
        mark("event loop to first frame");
    } else {
        mark(&format!("frame {} render", n - 1));
    }
    if n >= REPORT_AFTER_FRAMES {
        report();
    }
}

fn report() {
    let mut guard = PROFILE.lock().unwrap();
    let Some(profile) = guard.as_mut() else {
        return;
    };
    profile.reported = true;
    let total_ms = (Instant::now() - profile.started).as_secs_f64() * 1e3;
    let mut ranked: Vec<&(String, f64)> = profile.phases.iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut text = format!("startup profile: {total_ms:.0} ms to the start of frame {REPORT_AFTER_FRAMES}\n");
    for (label, ms) in &ranked {
        text.push_str(&format!(
            "  {ms:9.1} ms  {:5.1}%  {label}\n",
            100.0 * ms / total_ms
        ));
    }
    // The app redirects stderr, so keep the table where it can be read.
    let _ = std::fs::write("test-runs/startup_profile_latest.txt", &text);
    eprintln!("{text}");
    for (label, ms) in &profile.phases {
        tracing::info!(target: "planet::startup_profile", phase = label.as_str(), ms = *ms, "startup phase");
    }
    tracing::info!(target: "planet::startup_profile", total_ms, "startup profile total");
}
