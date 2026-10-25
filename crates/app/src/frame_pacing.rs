//! Per-frame wall-clock intervals, so p99 / 1% lows and stalls are measured
//! from every frame rather than the 0.5 s logging cadence. One `f32` push per
//! frame; the summary is rewritten to `test-runs/frame_pacing_latest.txt`
//! every `REWRITE_EVERY` samples and by `write_summary` at exit.

use std::{
    path::PathBuf,
    sync::{mpsc, Mutex, OnceLock},
};

/// Frames dropped at the start (pipeline compilation, first uploads).
const WARMUP_FRAMES: u32 = 120;
const MAX_SAMPLES: usize = 1 << 20;
const REWRITE_EVERY: usize = 600;
const PATH: &str = "test-runs/frame_pacing_latest.txt";

struct Pacing {
    frames_seen: u32,
    intervals_ms: Vec<f32>,
    previous_excluded: bool,
    /// (frame number, interval ms, previous frame was a capture) for every
    /// frame over `STALL_MS`, so a stall can be tied to what happened there.
    stalls: Vec<(u32, f32, bool)>,
}

const STALL_MS: f32 = 100.0;

static PACING: Mutex<Pacing> = Mutex::new(Pacing {
    frames_seen: 0,
    intervals_ms: Vec::new(),
    previous_excluded: false,
    stalls: Vec::new(),
});

struct SummaryRequest {
    intervals_ms: Vec<f32>,
    stalls: Vec<(u32, f32, bool)>,
    done: Option<mpsc::Sender<()>>,
}

static SUMMARY_WORKER: OnceLock<mpsc::Sender<SummaryRequest>> = OnceLock::new();

fn spawn_summary_worker(path: PathBuf) -> mpsc::Sender<SummaryRequest> {
    let (sender, receiver) = mpsc::channel::<SummaryRequest>();
    std::thread::Builder::new()
        .name("frame-pacing-summary".into())
        .spawn(move || {
            while let Ok(request) = receiver.recv() {
                let text = summary(&request.intervals_ms, &request.stalls);
                let _ = std::fs::write(&path, text);
                if let Some(done) = request.done {
                    let _ = done.send(());
                }
            }
        })
        .expect("frame-pacing summary worker must start");
    sender
}

fn queue_summary(request: SummaryRequest) {
    let worker = SUMMARY_WORKER.get_or_init(|| spawn_summary_worker(PATH.into()));
    if let Err(error) = worker.send(request) {
        // Still produce a final report if the worker has unexpectedly stopped.
        let request = error.0;
        let _ = std::fs::write(PATH, summary(&request.intervals_ms, &request.stalls));
        if let Some(done) = request.done {
            let _ = done.send(());
        }
    }
}

/// Records one frame interval. `excluded` marks frames that are not
/// representative (screenshot readback); they still count toward warm-up.
pub fn record(interval_ms: f32, excluded: bool) {
    let mut pacing = PACING.lock().unwrap();
    pacing.frames_seen = pacing.frames_seen.saturating_add(1);
    let after_capture = pacing.previous_excluded;
    pacing.previous_excluded = excluded;
    if interval_ms > STALL_MS && pacing.stalls.len() < 64 {
        let frame = pacing.frames_seen;
        pacing.stalls.push((frame, interval_ms, after_capture));
    }
    // The frame after a capture absorbs the screenshot readback wait too.
    if excluded || after_capture || pacing.frames_seen <= WARMUP_FRAMES || pacing.intervals_ms.len() >= MAX_SAMPLES {
        return;
    }
    pacing.intervals_ms.push(interval_ms);
    if pacing.intervals_ms.len() % REWRITE_EVERY == 0 {
        // A bounded snapshot is cheaper than sorting the entire growing
        // history and writing a file on the render thread every 600 frames.
        let request = SummaryRequest {
            intervals_ms: pacing.intervals_ms.clone(),
            stalls: pacing.stalls.clone(),
            done: None,
        };
        drop(pacing);
        queue_summary(request);
    }
}

pub fn write_summary() {
    let pacing = PACING.lock().unwrap();
    if pacing.intervals_ms.is_empty() {
        return;
    }
    let (done, completed) = mpsc::channel();
    let request = SummaryRequest {
        intervals_ms: pacing.intervals_ms.clone(),
        stalls: pacing.stalls.clone(),
        done: Some(done),
    };
    drop(pacing);
    queue_summary(request);
    let _ = completed.recv();
}

fn percentile(sorted: &[f32], fraction: f64) -> f32 {
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn summary(intervals_ms: &[f32], stalls: &[(u32, f32, bool)]) -> String {
    let mut sorted = intervals_ms.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let count = sorted.len();
    let mean = sorted.iter().map(|&v| v as f64).sum::<f64>() / count as f64;
    // "1% low" FPS: the mean rate over the slowest 1% of frames.
    let slowest = &sorted[count - (count / 100).max(1)..];
    let one_percent_low_fps =
        1000.0 / (slowest.iter().map(|&v| v as f64).sum::<f64>() / slowest.len() as f64);
    let over_33 = sorted.iter().filter(|&&v| v > 33.4).count();
    let over_50 = sorted.iter().filter(|&&v| v > 50.0).count();
    let mut stall_lines = String::new();
    for (frame, interval, after_capture) in stalls {
        stall_lines.push_str(&format!(
            "stall frame {frame} {interval:.1}ms{}\n",
            if *after_capture { " (follows a capture frame)" } else { "" }
        ));
    }
    format!(
        "{stall_lines}frames {count} (first {WARMUP_FRAMES}, captures and the frame after each excluded)\n\
         mean_ms {mean:.3}\np50_ms {:.3}\np95_ms {:.3}\np99_ms {:.3}\nmax_ms {:.3}\n\
         one_percent_low_fps {one_percent_low_fps:.1}\nframes_over_33ms {over_33}\nframes_over_50ms {over_50}\n",
        percentile(&sorted, 0.5),
        percentile(&sorted, 0.95),
        percentile(&sorted, 0.99),
        sorted[count - 1],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_reports_percentiles_and_one_percent_low() {
        let mut values = vec![10.0f32; 99];
        values.push(100.0);
        let text = summary(&values, &[]);
        assert!(text.contains("p50_ms 10.000"), "{text}");
        assert!(text.contains("max_ms 100.000"), "{text}");
        assert!(text.contains("one_percent_low_fps 10.0"), "{text}");
        assert!(text.contains("frames_over_50ms 1"), "{text}");
    }

    #[test]
    fn background_summaries_remain_ordered_and_final_flush_is_complete() {
        let path = std::env::temp_dir().join(format!(
            "planet-frame-pacing-test-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let worker = spawn_summary_worker(path.clone());
        let first = vec![10.0; REWRITE_EVERY];
        let mut last = first.clone();
        last.push(100.0);
        worker
            .send(SummaryRequest {
                intervals_ms: first,
                stalls: Vec::new(),
                done: None,
            })
            .unwrap();
        let (done, completed) = mpsc::channel();
        worker
            .send(SummaryRequest {
                intervals_ms: last.clone(),
                stalls: vec![(42, 100.0, true)],
                done: Some(done),
            })
            .unwrap();
        completed.recv().unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            summary(&last, &[(42, 100.0, true)])
        );
    }
}
