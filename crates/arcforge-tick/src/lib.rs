//! Fixed-timestep tick scheduler for Arcforge.
//!
//! Provides configurable tick rates (1–128 Hz) for real-time game loops
//! with budget monitoring, overrun handling, and pause/resume support.
//!
//! # Event-driven mode
//!
//! When `tick_rate_hz` is 0, the scheduler enters event-driven mode and
//! [`TickScheduler::wait_for_tick`] pends forever. This is the correct
//! behavior for turn-based games that only react to player messages.
//!
//! # Integration
//!
//! The scheduler is designed to sit inside a room actor's `tokio::select!` loop:
//!
//! ```ignore
//! loop {
//!     tokio::select! {
//!         Some(cmd) = cmd_rx.recv() => { /* handle commands */ }
//!         tick_info = scheduler.wait_for_tick() => {
//!             let msgs = G::tick(&mut state, tick_info.dt);
//!             scheduler.record_tick_end();
//!         }
//!     }
//! }
//! ```

use std::time::{Duration, Instant};

use rand::Rng;
use tokio::time::{self, Instant as TokioInstant};
use tracing::{debug, trace, warn};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// What to do when a tick takes longer than its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickPolicy {
    /// Skip the missed tick(s) and resume from now.
    /// Safest default — prevents death spirals.
    Skip,
    /// Run up to `max_catchup` extra ticks immediately.
    /// Use only when deterministic simulation replay is required.
    CatchUp {
        /// Hard cap on consecutive catch-up ticks to prevent death spirals.
        /// Skeptekh warning: without a cap, catch-up creates exponential CPU usage.
        max_catchup: u32,
    },
    /// Drop the overrun entirely — don't adjust timing.
    /// The next tick fires at its originally scheduled time.
    Drop,
}

impl Default for TickPolicy {
    fn default() -> Self {
        Self::Skip
    }
}

/// Full configuration for the tick scheduler.
#[derive(Debug, Clone)]
pub struct TickConfig {
    /// Tick rate in Hz. 0 = event-driven (tick never fires).
    pub tick_rate_hz: u32,
    /// Overrun handling policy.
    pub policy: TickPolicy,
    /// Budget warning threshold (0.0–1.0). Default: 0.80 (80%).
    /// A tracing warning is emitted when tick execution exceeds this
    /// fraction of the tick budget.
    pub budget_warn_threshold: f64,
    /// Budget critical threshold (0.0–1.0). Default: 1.0 (100%).
    pub budget_critical_threshold: f64,
    /// Enable per-tick metrics collection. Adds minor overhead.
    /// At 60 Hz × 500 rooms = 30 K updates/s — acceptable on modern hardware
    /// but disable if profiling shows contention.
    pub metrics_enabled: bool,
    /// Random jitter (0–max µs) added to the *first* tick to desynchronize
    /// rooms created at the same instant (thundering-herd mitigation).
    pub initial_jitter_us: u64,
}

impl Default for TickConfig {
    fn default() -> Self {
        Self {
            tick_rate_hz: 0,
            policy: TickPolicy::default(),
            budget_warn_threshold: 0.80,
            budget_critical_threshold: 1.0,
            metrics_enabled: true,
            initial_jitter_us: 2_000, // 0–2 ms default jitter
        }
    }
}

impl TickConfig {
    /// Maximum supported tick rate.
    pub const MAX_TICK_RATE_HZ: u32 = 128;

    /// Create a config for a specific tick rate with sensible defaults.
    pub fn with_rate(tick_rate_hz: u32) -> Self {
        Self {
            tick_rate_hz,
            ..Default::default()
        }
    }

    /// Clamp and fix any out-of-range values so the config is safe to use.
    ///
    /// Called automatically by [`TickScheduler::new`]. Rules:
    /// - `tick_rate_hz` capped to [`Self::MAX_TICK_RATE_HZ`] (0 is allowed for event-driven).
    /// - Thresholds clamped to `0.0..=1.0`.
    /// - `budget_warn_threshold` forced ≤ `budget_critical_threshold`.
    pub fn validated(mut self) -> Self {
        if self.tick_rate_hz > Self::MAX_TICK_RATE_HZ {
            warn!(
                rate = self.tick_rate_hz,
                max = Self::MAX_TICK_RATE_HZ,
                "tick_rate_hz exceeds maximum — clamping"
            );
            self.tick_rate_hz = Self::MAX_TICK_RATE_HZ;
        }
        self.budget_warn_threshold = self.budget_warn_threshold.clamp(0.0, 1.0);
        self.budget_critical_threshold = self.budget_critical_threshold.clamp(0.0, 1.0);
        if self.budget_warn_threshold > self.budget_critical_threshold {
            self.budget_warn_threshold = self.budget_critical_threshold;
        }
        self
    }

    /// Duration of a single tick. Returns `None` for event-driven mode.
    pub fn tick_duration(&self) -> Option<Duration> {
        if self.tick_rate_hz == 0 {
            None
        } else {
            Some(Duration::from_secs_f64(1.0 / self.tick_rate_hz as f64))
        }
    }
}

// ---------------------------------------------------------------------------
// Tick info (returned to caller each tick)
// ---------------------------------------------------------------------------

/// Information about a completed tick, returned by [`TickScheduler::wait_for_tick`].
#[derive(Debug, Clone)]
pub struct TickInfo {
    /// Monotonically increasing tick number (starts at 1).
    pub tick: u64,
    /// Fixed delta time for this tick (always `1 / tick_rate`).
    /// Game logic should use this, not wall-clock elapsed time,
    /// to keep simulation deterministic.
    pub dt: Duration,
    /// `true` if this tick fired late (scheduler detected overrun).
    pub overrun: bool,
    /// How many ticks were skipped due to overrun (0 in normal operation).
    pub ticks_skipped: u64,
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Runtime metrics for the tick scheduler.
///
/// Updated after each tick when `metrics_enabled` is true.
/// All timing values refer to the *game logic* execution time
/// reported via [`TickScheduler::record_tick_end`].
#[derive(Debug, Clone)]
pub struct TickMetrics {
    /// Total ticks executed.
    pub total_ticks: u64,
    /// Total overruns detected.
    pub total_overruns: u64,
    /// Total ticks skipped (from Skip/CatchUp policies).
    pub total_skipped: u64,
    /// Exponential moving average of tick execution time (α = 0.1).
    pub avg_tick_time: Duration,
    /// Maximum tick execution time observed.
    pub max_tick_time: Duration,
    /// Current budget utilization (0.0–∞). >1.0 means overrun.
    pub budget_utilization: f64,
}

impl Default for TickMetrics {
    fn default() -> Self {
        Self {
            total_ticks: 0,
            total_overruns: 0,
            total_skipped: 0,
            avg_tick_time: Duration::ZERO,
            max_tick_time: Duration::ZERO,
            budget_utilization: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Fixed-timestep tick scheduler.
///
/// Drives the game loop for a single room. One `TickScheduler` per room actor.
pub struct TickScheduler {
    config: TickConfig,
    tick_duration: Option<Duration>,
    tick_count: u64,
    /// When the next tick should fire (Tokio instant for `sleep_until`).
    next_tick: Option<TokioInstant>,
    /// Wall-clock instant when the last tick's game logic started.
    /// Set by `wait_for_tick`, consumed by `record_tick_end`.
    tick_start: Option<Instant>,
    paused: bool,
    metrics: TickMetrics,
}

impl TickScheduler {
    /// Create a new scheduler from config.
    ///
    /// The first tick is scheduled with optional jitter to prevent
    /// thundering-herd synchronization across rooms.
    pub fn new(config: TickConfig) -> Self {
        let config = config.validated();
        let tick_duration = config.tick_duration();

        // Schedule first tick with jitter to desynchronize rooms.
        let next_tick = tick_duration.map(|d| {
            let jitter = if config.initial_jitter_us > 0 {
                let us = rand::rng().random_range(0..config.initial_jitter_us);
                Duration::from_micros(us)
            } else {
                Duration::ZERO
            };
            TokioInstant::now() + d + jitter
        });

        if config.tick_rate_hz == 0 {
            debug!("tick scheduler created in event-driven mode (no tick loop)");
        } else {
            debug!(
                rate_hz = config.tick_rate_hz,
                budget_ms = ?tick_duration.map(|d| d.as_secs_f64() * 1000.0),
                policy = ?config.policy,
                "tick scheduler created"
            );
        }

        Self {
            config,
            tick_duration,
            tick_count: 0,
            next_tick,
            tick_start: None,
            paused: false,
            metrics: TickMetrics::default(),
        }
    }

    /// Create a scheduler for a specific tick rate with default settings.
    pub fn with_rate(tick_rate_hz: u32) -> Self {
        Self::new(TickConfig::with_rate(tick_rate_hz))
    }

    /// Wait until the next tick is due. Returns [`TickInfo`] for the tick.
    ///
    /// In event-driven mode (`tick_rate_hz == 0`) or when paused, this
    /// future pends forever — it will never resolve on its own, but
    /// `tokio::select!` will still process other branches.
    pub async fn wait_for_tick(&mut self) -> TickInfo {
        // Event-driven or paused: pend forever.
        let (next, tick_dur) = match (self.next_tick, self.tick_duration) {
            (Some(next), Some(dur)) if !self.paused => (next, dur),
            _ => {
                // This future never completes — select! handles other branches.
                std::future::pending::<()>().await;
                unreachable!()
            }
        };

        time::sleep_until(next).await;

        let now = TokioInstant::now();
        self.tick_count += 1;
        self.tick_start = Some(Instant::now());

        // Detect overrun: did we wake up significantly late?
        let late_by = now.saturating_duration_since(next);
        let overrun = late_by > tick_dur / 10; // >10% late = overrun
        let mut ticks_skipped = 0u64;

        // Schedule next tick based on policy.
        self.next_tick = Some(match self.config.policy {
            TickPolicy::Skip => {
                if overrun {
                    ticks_skipped = late_by.as_nanos() as u64 / tick_dur.as_nanos() as u64;
                    if ticks_skipped > 0 {
                        warn!(
                            tick = self.tick_count,
                            skipped = ticks_skipped,
                            late_ms = late_by.as_secs_f64() * 1000.0,
                            "tick overrun — skipping ahead"
                        );
                    }
                }
                // Always schedule from now, not from the missed deadline.
                now + tick_dur
            }
            TickPolicy::CatchUp { max_catchup } => {
                if overrun {
                    let behind = late_by.as_nanos() as u64 / tick_dur.as_nanos() as u64;
                    ticks_skipped = behind.saturating_sub(max_catchup as u64);
                    if behind > 0 {
                        warn!(
                            tick = self.tick_count,
                            behind,
                            catching_up = behind.min(max_catchup as u64),
                            skipping = ticks_skipped,
                            "tick overrun — catch-up capped at {max_catchup}"
                        );
                    }
                    // Schedule next tick immediately for catch-up, but cap it.
                    if behind <= max_catchup as u64 {
                        next + tick_dur
                    } else {
                        now + tick_dur
                    }
                } else {
                    next + tick_dur
                }
            }
            TickPolicy::Drop => {
                if overrun {
                    warn!(
                        tick = self.tick_count,
                        late_ms = late_by.as_secs_f64() * 1000.0,
                        "tick overrun — dropping (next tick at original schedule)"
                    );
                }
                // Keep the original cadence regardless of overrun.
                next + tick_dur
            }
        });

        if overrun {
            self.metrics.total_overruns += 1;
        }
        self.metrics.total_skipped += ticks_skipped;
        self.metrics.total_ticks += 1;

        trace!(tick = self.tick_count, overrun, "tick fired");

        TickInfo {
            tick: self.tick_count,
            dt: tick_dur,
            overrun,
            ticks_skipped,
        }
    }

    /// Record that the game logic for the current tick has finished.
    ///
    /// Call this after `GameLogic::tick()` returns to enable budget
    /// monitoring and metrics. If not called, budget warnings won't fire.
    pub fn record_tick_end(&mut self) {
        let Some(start) = self.tick_start.take() else {
            return;
        };
        let elapsed = start.elapsed();

        if let Some(budget) = self.tick_duration {
            let utilization = elapsed.as_secs_f64() / budget.as_secs_f64();
            self.metrics.budget_utilization = utilization;

            if utilization >= self.config.budget_critical_threshold {
                warn!(
                    tick = self.tick_count,
                    elapsed_ms = elapsed.as_secs_f64() * 1000.0,
                    budget_ms = budget.as_secs_f64() * 1000.0,
                    utilization_pct = format!("{:.1}", utilization * 100.0),
                    "CRITICAL: tick exceeded budget"
                );
            } else if utilization >= self.config.budget_warn_threshold {
                warn!(
                    tick = self.tick_count,
                    elapsed_ms = elapsed.as_secs_f64() * 1000.0,
                    budget_ms = budget.as_secs_f64() * 1000.0,
                    utilization_pct = format!("{:.1}", utilization * 100.0),
                    "tick approaching budget limit"
                );
            }
        }

        // Update metrics.
        if self.config.metrics_enabled {
            if elapsed > self.metrics.max_tick_time {
                self.metrics.max_tick_time = elapsed;
            }
            // Exponential moving average (α = 0.1).
            let alpha = 0.1;
            let prev = self.metrics.avg_tick_time.as_secs_f64();
            let curr = elapsed.as_secs_f64();
            self.metrics.avg_tick_time =
                Duration::from_secs_f64(prev * (1.0 - alpha) + curr * alpha);
        }
    }

    /// Pause the tick loop. `wait_for_tick` will pend until [`resume`](Self::resume) is called.
    ///
    /// Safe to call multiple times (idempotent).
    pub fn pause(&mut self) {
        if !self.paused {
            self.paused = true;
            debug!(tick = self.tick_count, "tick scheduler paused");
        }
    }

    /// Resume the tick loop after a pause.
    ///
    /// Resets the next-tick deadline to `now + tick_duration` to avoid
    /// a burst of catch-up ticks from the time spent paused.
    pub fn resume(&mut self) {
        if self.paused {
            self.paused = false;
            if let Some(dur) = self.tick_duration {
                self.next_tick = Some(TokioInstant::now() + dur);
            }
            debug!(tick = self.tick_count, "tick scheduler resumed");
        }
    }

    /// Whether the scheduler is currently paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Whether this scheduler is in event-driven mode (tick rate = 0).
    pub fn is_event_driven(&self) -> bool {
        self.tick_duration.is_none()
    }

    /// Current tick count.
    pub fn tick_count(&self) -> u64 {
        self.tick_count
    }

    /// Snapshot of current metrics.
    pub fn metrics(&self) -> &TickMetrics {
        &self.metrics
    }

    /// The configured tick rate in Hz.
    pub fn tick_rate_hz(&self) -> u32 {
        self.config.tick_rate_hz
    }

    /// The fixed tick duration, or `None` for event-driven mode.
    pub fn tick_duration(&self) -> Option<Duration> {
        self.tick_duration
    }
}
