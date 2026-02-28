//! Integration tests for the fixed-timestep tick scheduler.
//!
//! Uses `tokio::time::pause()` to control time deterministically.
//! All tests run with auto-advanced time so `sleep_until` resolves
//! instantly when we advance the clock.

use std::time::Duration;

use arcforge_tick::{TickConfig, TickPolicy, TickScheduler};

// =========================================================================
// Helpers
// =========================================================================

fn config_20hz() -> TickConfig {
    TickConfig::with_rate(20)
}

fn config_event_driven() -> TickConfig {
    TickConfig::with_rate(0)
}

// =========================================================================
// TickConfig
// =========================================================================

#[test]
fn test_default_config_is_event_driven() {
    let cfg = TickConfig::default();
    assert_eq!(cfg.tick_rate_hz, 0);
    assert_eq!(cfg.tick_duration(), None);
}

#[test]
fn test_with_rate_sets_duration() {
    let cfg = TickConfig::with_rate(20);
    let dur = cfg.tick_duration().unwrap();
    assert_eq!(dur, Duration::from_millis(50));
}

#[test]
fn test_tick_duration_60hz() {
    let cfg = TickConfig::with_rate(60);
    let dur = cfg.tick_duration().unwrap();
    // 1/60 ≈ 16.666ms
    let expected = Duration::from_secs_f64(1.0 / 60.0);
    assert_eq!(dur, expected);
}

// =========================================================================
// Scheduler creation and accessors
// =========================================================================

#[test]
fn test_scheduler_initial_state() {
    let s = TickScheduler::new(config_20hz());
    assert_eq!(s.tick_count(), 0);
    assert_eq!(s.tick_rate_hz(), 20);
    assert!(!s.is_event_driven());
    assert!(!s.is_paused());
    assert_eq!(s.tick_duration(), Some(Duration::from_millis(50)));
}

#[test]
fn test_scheduler_event_driven() {
    let s = TickScheduler::new(config_event_driven());
    assert!(s.is_event_driven());
    assert_eq!(s.tick_duration(), None);
    assert_eq!(s.tick_rate_hz(), 0);
}

#[test]
fn test_with_rate_constructor() {
    let s = TickScheduler::with_rate(10);
    assert_eq!(s.tick_rate_hz(), 10);
    assert_eq!(s.tick_duration(), Some(Duration::from_millis(100)));
}

// =========================================================================
// Tick firing
// =========================================================================

#[tokio::test(start_paused = true)]
async fn test_wait_for_tick_fires_and_increments() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        ..config_20hz()
    });

    let info = s.wait_for_tick().await;
    assert_eq!(info.tick, 1);
    assert_eq!(info.dt, Duration::from_millis(50));
    assert!(!info.overrun);
    assert_eq!(info.ticks_skipped, 0);
    assert_eq!(s.tick_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn test_multiple_ticks_increment_monotonically() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        ..config_20hz()
    });

    for expected in 1..=5 {
        let info = s.wait_for_tick().await;
        assert_eq!(info.tick, expected);
    }
    assert_eq!(s.tick_count(), 5);
}

#[tokio::test(start_paused = true)]
async fn test_dt_is_always_fixed() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        ..config_20hz()
    });

    let expected_dt = Duration::from_millis(50);
    for _ in 0..3 {
        let info = s.wait_for_tick().await;
        assert_eq!(info.dt, expected_dt);
    }
}

// =========================================================================
// Event-driven mode pends forever
// =========================================================================

#[tokio::test(start_paused = true)]
async fn test_event_driven_never_fires() {
    let mut s = TickScheduler::new(config_event_driven());

    // wait_for_tick should never resolve — select! with a timeout proves it.
    let result = tokio::time::timeout(Duration::from_secs(5), s.wait_for_tick()).await;
    assert!(result.is_err(), "event-driven scheduler should pend forever");
}

// =========================================================================
// Pause / Resume
// =========================================================================

#[tokio::test(start_paused = true)]
async fn test_pause_prevents_ticks() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        ..config_20hz()
    });

    // Fire one tick to confirm it works.
    s.wait_for_tick().await;
    assert_eq!(s.tick_count(), 1);

    s.pause();
    assert!(s.is_paused());

    // Should not fire while paused.
    let result = tokio::time::timeout(Duration::from_secs(1), s.wait_for_tick()).await;
    assert!(result.is_err(), "paused scheduler should pend");
}

#[tokio::test(start_paused = true)]
async fn test_resume_allows_ticks_again() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        ..config_20hz()
    });

    s.wait_for_tick().await;
    s.pause();
    s.resume();
    assert!(!s.is_paused());

    let info = s.wait_for_tick().await;
    assert_eq!(info.tick, 2);
}

#[tokio::test]
async fn test_pause_resume_idempotent() {
    let mut s = TickScheduler::new(config_20hz());

    // Multiple pauses shouldn't panic or change state.
    s.pause();
    s.pause();
    assert!(s.is_paused());

    s.resume();
    s.resume();
    assert!(!s.is_paused());
}

// =========================================================================
// Metrics
// =========================================================================

#[test]
fn test_initial_metrics_are_zero() {
    let s = TickScheduler::new(config_20hz());
    let m = s.metrics();
    assert_eq!(m.total_ticks, 0);
    assert_eq!(m.total_overruns, 0);
    assert_eq!(m.total_skipped, 0);
    assert_eq!(m.avg_tick_time, Duration::ZERO);
    assert_eq!(m.max_tick_time, Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn test_metrics_total_ticks_increments() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        ..config_20hz()
    });

    for _ in 0..3 {
        s.wait_for_tick().await;
        s.record_tick_end();
    }

    assert_eq!(s.metrics().total_ticks, 3);
}

#[tokio::test(start_paused = true)]
async fn test_record_tick_end_without_wait_is_noop() {
    let mut s = TickScheduler::new(config_20hz());

    // Calling record_tick_end without a prior wait_for_tick should not panic.
    s.record_tick_end();
    assert_eq!(s.metrics().total_ticks, 0);
}

#[tokio::test(start_paused = true)]
async fn test_metrics_max_tick_time_tracked() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        ..config_20hz()
    });

    // record_tick_end uses std::time::Instant (wall clock), not tokio time.
    // We can't mock it, but we can verify it records *something* > ZERO.
    s.wait_for_tick().await;
    s.record_tick_end();

    s.wait_for_tick().await;
    // Burn a tiny bit of real wall-clock time.
    std::thread::sleep(Duration::from_micros(50));
    s.record_tick_end();

    // max_tick_time should have been updated (non-zero after thread::sleep).
    assert!(s.metrics().max_tick_time > Duration::ZERO);
}

// =========================================================================
// Budget utilization
// =========================================================================

#[tokio::test(start_paused = true)]
async fn test_budget_utilization_under_budget() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        ..config_20hz() // 50ms budget
    });

    s.wait_for_tick().await;
    // record_tick_end uses std::time::Instant (wall clock), so we need
    // real wall-clock time to elapse for a meaningful utilization value.
    std::thread::sleep(Duration::from_micros(50));
    s.record_tick_end();

    let util = s.metrics().budget_utilization;
    assert!(util > 0.0, "utilization should be non-zero after real work");
    assert!(util < 1.0, "utilization should be under budget");
}

// =========================================================================
// Tick policies
// =========================================================================

#[test]
fn test_default_policy_is_skip() {
    let cfg = TickConfig::default();
    assert_eq!(cfg.policy, TickPolicy::Skip);
}

#[test]
fn test_policy_catchup_stores_max() {
    let policy = TickPolicy::CatchUp { max_catchup: 5 };
    match policy {
        TickPolicy::CatchUp { max_catchup } => assert_eq!(max_catchup, 5),
        _ => panic!("expected CatchUp"),
    }
}

#[tokio::test(start_paused = true)]
async fn test_drop_policy_normal_tick() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        policy: TickPolicy::Drop,
        ..config_20hz()
    });

    let info = s.wait_for_tick().await;
    assert!(!info.overrun);
    assert_eq!(info.ticks_skipped, 0);
}

#[tokio::test(start_paused = true)]
async fn test_catchup_policy_normal_tick() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        policy: TickPolicy::CatchUp { max_catchup: 3 },
        ..config_20hz()
    });

    let info = s.wait_for_tick().await;
    assert!(!info.overrun);
    assert_eq!(info.ticks_skipped, 0);
}

// =========================================================================
// Metrics disabled
// =========================================================================

#[tokio::test(start_paused = true)]
async fn test_metrics_disabled_skips_avg_update() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        metrics_enabled: false,
        ..config_20hz()
    });

    s.wait_for_tick().await;
    tokio::time::advance(Duration::from_millis(10)).await;
    s.record_tick_end();

    // avg and max should stay at zero when metrics are disabled.
    assert_eq!(s.metrics().avg_tick_time, Duration::ZERO);
    assert_eq!(s.metrics().max_tick_time, Duration::ZERO);
}

// =========================================================================
// Integration: select! loop pattern (mirrors real room usage)
// =========================================================================

#[tokio::test(start_paused = true)]
async fn test_select_loop_pattern() {
    let mut s = TickScheduler::new(TickConfig {
        initial_jitter_us: 0,
        ..config_20hz()
    });

    let (tx, mut rx) = tokio::sync::mpsc::channel::<&str>(10);

    // Simulate: 3 ticks fire, then a "stop" command arrives.
    let tx2 = tx.clone();
    tokio::spawn(async move {
        // Send stop after ~150ms (3 ticks at 20Hz = 50ms each).
        tokio::time::sleep(Duration::from_millis(160)).await;
        tx2.send("stop").await.ok();
    });

    let mut ticks_fired = 0u64;
    loop {
        tokio::select! {
            Some(cmd) = rx.recv() => {
                assert_eq!(cmd, "stop");
                break;
            }
            info = s.wait_for_tick() => {
                ticks_fired += 1;
                s.record_tick_end();
                assert_eq!(info.tick, ticks_fired);
            }
        }
    }

    assert!(ticks_fired >= 3, "expected at least 3 ticks, got {ticks_fired}");
}
