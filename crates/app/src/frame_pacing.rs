//! Per-frame wall-clock intervals, so p99 / 1% lows and stalls are measured
//! from every frame rather than the 0.5 s logging cadence. One `f32` push per
//! frame; the summary is rewritten to `test-runs/frame_pacing_latest.txt`
//! every `REWRITE_EVERY` samples and by `write_summary` at exit.

use std::sync::Mutex;

/// Frames dropped at the start (pipeline compilation, first uploads).
const WARMUP_FRAMES: u32 = 120;
const MAX_SAMPLES: usize = 1 << 20;
const REWRITE_EVERY: usize = 600;
const PATH: &str = "test-runs/frame_pacing_latest.txt";

struct Pacing {
    frames_seen: u32,
    intervals_ms: Vec<f32>,
}

static PACING: Mutex<Pacing> = Mutex::new(Pacing {
    frames_seen: 0,
    intervals_ms: Vec::new(),
});

/// Records one frame interval. `excluded` marks frames that are not
/// representative (screenshot readback); they still count toward warm-up.
pub fn record(interval_ms: f32, excluded: bool) {
    let mut pacing = PACING.lock().unwrap();
    pacing.frames_seen = pacing.frames_seen.saturating_add(1);
    if excluded || pacing.frames_seen <= WARMUP_FRAMES || pacing.intervals_ms.len() >= MAX_SAMPLES {
        return;
    }
    pacing.intervals_ms.push(interval_ms);
    if pacing.intervals_ms.len() % REWRITE_EVERY == 0 {
        let text = summary(&pacing.intervals_ms);
        drop(pacing);
        let _ = std::fs::write(PATH, text);
    }
}

pub fn write_summary() {
    let pacing = PACING.lock().unwrap();
    if pacing.intervals_ms.is_empty() {
        return;
    }
    let text = summary(&pacing.intervals_ms);
    drop(pacing);
    let _ = std::fs::write(PATH, text);
}

fn percentile(sorted: &[f32], fraction: f64) -> f32 {
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn summary(intervals_ms: &[f32]) -> String {
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
    format!(
        "frames {count} (first {WARMUP_FRAMES} and captures excluded)\n\
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
        let text = summary(&values);
        assert!(text.contains("p50_ms 10.000"), "{text}");
        assert!(text.contains("max_ms 100.000"), "{text}");
        assert!(text.contains("one_percent_low_fps 10.0"), "{text}");
        assert!(text.contains("frames_over_50ms 1"), "{text}");
    }
}
