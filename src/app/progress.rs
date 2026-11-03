//! Progress display of `install` and `update` on stderr: one line per finished phase; on a live
//! terminal the running phase animates a spinner with its elapsed time, and a download a smooth
//! bar with bytes, speed and remaining time. Off a terminal (or with `TERM=dumb`) nothing is
//! drawn: the command's result lines on stdout carry the same facts for logs and agents.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// Eighth blocks for the leading cell of the bar.
const PARTIAL: [&str; 7] = ["▏", "▎", "▍", "▌", "▋", "▊", "▉"];
const BAR_CELLS: usize = 24;
const FRAME: Duration = Duration::from_millis(80);
const LABEL_WIDTH: usize = 11;
/// Speed samples at least this far apart; the newest sample weighs `SMOOTHING`.
const RATE_SAMPLE: Duration = Duration::from_millis(250);
const SMOOTHING: f64 = 0.3;
/// Without new bytes for this long the speed and remaining time give way to "stalled".
const STALL: Duration = Duration::from_secs(2);

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";
const CLEAR_LINE: &str = "\r\x1b[2K";

#[derive(Default)]
struct Running {
    label: String,
    started: Option<Instant>,
    done: u64,
    total: u64,
    rate_at: Option<Instant>,
    rate_done: u64,
    speed: f64,
    last_read: Option<Instant>,
    frame: usize,
}

/// Phase reporter; draws only when `live`.
pub struct Progress {
    live: bool,
    color: bool,
    state: Arc<Mutex<Running>>,
    quit: Arc<AtomicBool>,
    animation: Option<JoinHandle<()>>,
}

impl Progress {
    /// Reporter on stderr, live when stderr is a terminal and `TERM` is not `dumb`.
    pub fn stderr() -> Self {
        let terminal = std::io::stderr().is_terminal()
            && std::env::var("TERM").map_or(true, |term| !term.eq_ignore_ascii_case("dumb"));
        let color = terminal && std::env::var_os("NO_COLOR").is_none();
        Self::new(terminal, color)
    }

    pub fn new(live: bool, color: bool) -> Self {
        Self {
            live,
            color,
            state: Arc::new(Mutex::new(Running::default())),
            quit: Arc::new(AtomicBool::new(false)),
            animation: None,
        }
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    /// Starts a phase (`Download`, `Verify`, ...), ending none.
    pub fn start(&mut self, label: &str) {
        let now = Instant::now();
        if let Ok(mut state) = self.state.lock() {
            *state = Running {
                label: label.to_owned(),
                started: Some(now),
                rate_at: Some(now),
                last_read: Some(now),
                ..Running::default()
            };
        }
        if self.live && self.animation.is_none() {
            self.quit.store(false, Ordering::SeqCst);
            let state = Arc::clone(&self.state);
            let quit = Arc::clone(&self.quit);
            let color = self.color;
            self.animation = Some(std::thread::spawn(move || {
                while !quit.load(Ordering::SeqCst) {
                    if let Ok(mut state) = state.lock() {
                        state.frame += 1;
                        draw_running(&state, color);
                    }
                    std::thread::sleep(FRAME);
                }
            }));
        }
    }

    /// Bytes transferred so far in the running phase (`total` 0 when unknown).
    pub fn bytes(&self, done: u64, total: u64) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let now = Instant::now();
        if done > state.done {
            state.last_read = Some(now);
        }
        state.done = done;
        state.total = total;
        if let Some(rate_at) = state.rate_at
            && now.duration_since(rate_at) >= RATE_SAMPLE
        {
            let sample =
                (done - state.rate_done) as f64 / now.duration_since(rate_at).as_secs_f64();
            state.speed = if state.speed == 0.0 {
                sample
            } else {
                SMOOTHING * sample + (1.0 - SMOOTHING) * state.speed
            };
            state.rate_at = Some(now);
            state.rate_done = done;
        }
    }

    /// Ends the running phase as passed with a detail (`3.1 MB · 2.4 MB/s`).
    pub fn pass(&mut self, detail: &str) {
        self.finish(true, detail);
    }

    /// Ends the running phase as failed.
    pub fn fail(&mut self, detail: &str) {
        self.finish(false, detail);
    }

    fn finish(&mut self, passed: bool, detail: &str) {
        self.stop_animation();
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.label.is_empty() {
            return;
        }
        // Off a terminal the result lines on stdout carry the same facts.
        if !self.live {
            *state = Running::default();
            return;
        }
        let mark = match (passed, self.color) {
            (true, true) => format!("{GREEN}✓{RESET}"),
            (true, false) => "✓".to_owned(),
            (false, true) => format!("{RED}✗{RESET}"),
            (false, false) => "✗".to_owned(),
        };
        let mut detail = home_relative(detail);
        if passed && state.total > 0 {
            let mut size = byte_count(state.total);
            if let Some(started) = state.started {
                let elapsed = started.elapsed().as_secs_f64();
                if self.live && elapsed >= 0.1 {
                    size.push_str(&format!(" · {}", rate(state.total as f64 / elapsed)));
                }
            }
            detail = if detail.is_empty() {
                size
            } else {
                format!("{size} · {detail}")
            };
        }
        let label = format!("{:<LABEL_WIDTH$}", state.label);
        let detail = if detail.is_empty() || !self.color {
            detail
        } else {
            format!("{DIM}{detail}{RESET}")
        };
        let prefix = if self.live { CLEAR_LINE } else { "" };
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, "{prefix}{mark} {label} {detail}");
        let _ = err.flush();
        *state = Running::default();
    }

    fn stop_animation(&mut self) {
        self.quit.store(true, Ordering::SeqCst);
        if let Some(handle) = self.animation.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        // A phase still running ended with the error the caller reports.
        let running = self.state.lock().is_ok_and(|state| !state.label.is_empty());
        if running {
            self.finish(false, "");
        }
        self.stop_animation();
    }
}

/// Paths below `$HOME` read as `~/...`.
fn home_relative(text: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => text.replace(&format!("{home}/"), "~/"),
        _ => text.to_owned(),
    }
}

fn draw_running(state: &Running, color: bool) {
    let Some(started) = state.started else {
        return;
    };
    let spinner = SPINNER[state.frame % SPINNER.len()];
    let spinner = if color {
        format!("{BOLD}{CYAN}{spinner}{RESET}")
    } else {
        spinner.to_owned()
    };
    let label = format!("{:<LABEL_WIDTH$}", state.label);
    let mut status = String::new();
    if state.total > 0 {
        status.push_str(&bar(state.done, state.total));
        status.push_str(&format!(
            "  {} / {}",
            byte_count(state.done),
            byte_count(state.total)
        ));
        let stalled = state.last_read.is_some_and(|last| last.elapsed() >= STALL);
        if stalled {
            status.push_str("  stalled");
        } else if state.speed > 0.0 {
            let remaining = (state.total.saturating_sub(state.done)) as f64 / state.speed;
            status.push_str(&format!(
                "  {}  {} left",
                rate(state.speed),
                duration(remaining)
            ));
        }
    } else if state.done > 0 {
        status.push_str(&byte_count(state.done));
    }
    let elapsed = started.elapsed();
    if elapsed >= Duration::from_secs(1) {
        status.push_str(&format!("  {}", duration(elapsed.as_secs_f64())));
    }
    let status = if color && !status.is_empty() {
        format!("{DIM}{status}{RESET}")
    } else {
        status
    };
    let mut err = std::io::stderr().lock();
    let _ = write!(err, "{CLEAR_LINE}{spinner} {label} {status}");
    let _ = err.flush();
}

/// A bar of `BAR_CELLS` cells, filled in eighths.
pub fn bar(done: u64, total: u64) -> String {
    let fraction = if total == 0 {
        0.0
    } else {
        (done as f64 / total as f64).clamp(0.0, 1.0)
    };
    let eighths = (fraction * (BAR_CELLS * 8) as f64).round() as usize;
    let full = eighths / 8;
    let mut out = "█".repeat(full);
    if full < BAR_CELLS {
        let rest = eighths % 8;
        if rest > 0 {
            out.push_str(PARTIAL[rest - 1]);
            out.push_str(&" ".repeat(BAR_CELLS - full - 1));
        } else {
            out.push_str(&" ".repeat(BAR_CELLS - full));
        }
    }
    format!("▕{out}▏")
}

pub fn byte_count(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn rate(bytes_per_second: f64) -> String {
    format!("{}/s", byte_count(bytes_per_second.max(0.0) as u64))
}

fn duration(seconds: f64) -> String {
    let seconds = seconds.clamp(0.0, 99.0 * 3600.0).round() as u64;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_below_home_read_relative() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        assert_eq!(home_relative(&format!("{home}/.local/bin")), "~/.local/bin");
        assert_eq!(home_relative("/elsewhere/bin"), "/elsewhere/bin");
    }

    #[test]
    fn bar_fills_in_eighths_and_keeps_its_width() {
        assert_eq!(bar(0, 100), format!("▕{}▏", " ".repeat(24)));
        assert_eq!(bar(100, 100), format!("▕{}▏", "█".repeat(24)));
        let half = bar(50, 100);
        assert_eq!(half.chars().count(), 26);
        assert!(half.contains(&"█".repeat(12)));
        // One sixteenth of the bar: one and a half cells.
        assert!(bar(1, 16).starts_with("▕█▌"));
    }

    #[test]
    fn sizes_rates_and_durations_read_compactly() {
        assert_eq!(byte_count(512), "512 B");
        assert_eq!(byte_count(3_100_000), "3.1 MB");
        assert_eq!(rate(2_400_000.0), "2.4 MB/s");
        assert_eq!(duration(42.0), "42s");
        assert_eq!(duration(125.0), "2m05s");
        assert_eq!(duration(7_300.0), "2h01m");
    }
}
