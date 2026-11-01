//! A small blocking rate limiter for online lookups, shared by all lookup
//! threads: a token bucket (burst + steady rate), a cap on requests in
//! flight, and a global back-off when the server says "slow down".

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Requests allowed back to back.
    pub burst: f64,
    /// Steady rate: one new request per this interval.
    pub interval: Duration,
    /// Maximum simultaneous HTTP requests.
    pub max_in_flight: usize,
}

impl Limits {
    /// Defaults for kaikki.org: burst of 4, then at most ~1.5 requests per
    /// second (90/min), at most 2 at a time.
    pub const ONLINE: Limits = Limits { burst: 4.0, interval: Duration::from_millis(650), max_in_flight: 2 };
}

struct State {
    tokens: f64,
    refilled: Instant,
    in_flight: usize,
    blocked_until: Option<Instant>,
}

pub struct RateLimiter {
    limits: Limits,
    state: Mutex<State>,
    cv: Condvar,
}

/// Returned by [`RateLimiter::acquire`]; releases the in-flight slot on drop.
pub struct Permit<'a>(&'a RateLimiter);

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap().in_flight -= 1;
        self.0.cv.notify_all();
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Cancelled;

impl RateLimiter {
    pub fn new(limits: Limits) -> Self {
        RateLimiter {
            limits,
            state: Mutex::new(State {
                tokens: limits.burst,
                refilled: Instant::now(),
                in_flight: 0,
                blocked_until: None,
            }),
            cv: Condvar::new(),
        }
    }

    fn refill(&self, s: &mut State, now: Instant) {
        let gained = now.duration_since(s.refilled).as_secs_f64() / self.limits.interval.as_secs_f64();
        s.tokens = (s.tokens + gained).min(self.limits.burst);
        s.refilled = now;
    }

    /// Block until a request may be made. `wanted` is polled while waiting;
    /// once it returns false the wait is abandoned (the user moved on).
    pub fn acquire(&self, wanted: impl Fn() -> bool) -> Result<Permit<'_>, Cancelled> {
        let mut s = self.state.lock().unwrap();
        loop {
            if !wanted() {
                return Err(Cancelled);
            }
            let now = Instant::now();
            self.refill(&mut s, now);
            let wait = match s.blocked_until {
                Some(t) if t > now => t - now,
                _ if s.tokens < 1.0 => {
                    self.limits.interval.mul_f64(1.0 - s.tokens).max(Duration::from_millis(1))
                }
                _ if s.in_flight >= self.limits.max_in_flight => Duration::from_millis(250),
                _ => {
                    s.tokens -= 1.0;
                    s.in_flight += 1;
                    return Ok(Permit(self));
                }
            };
            // Wake at least every 100 ms to notice cancellation.
            s = self.cv.wait_timeout(s, wait.min(Duration::from_millis(100))).unwrap().0;
        }
    }

    /// Stop all requests for `d` (e.g. after HTTP 429 / 503).
    pub fn back_off(&self, d: Duration) {
        let mut s = self.state.lock().unwrap();
        let until = Instant::now() + d;
        s.blocked_until = Some(s.blocked_until.map_or(until, |t| t.max(until)));
        s.tokens = 0.0;
    }

    /// Time left of a back-off, if any.
    pub fn blocked_for(&self) -> Option<Duration> {
        let s = self.state.lock().unwrap();
        s.blocked_until.and_then(|t| t.checked_duration_since(Instant::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: Limits = Limits { burst: 3.0, interval: Duration::from_millis(50), max_in_flight: 2 };

    #[test]
    fn burst_then_steady_rate() {
        let rl = RateLimiter::new(Limits { max_in_flight: 100, ..FAST });
        let t = Instant::now();
        for _ in 0..3 {
            drop(rl.acquire(|| true).unwrap());
        }
        assert!(t.elapsed() < Duration::from_millis(20), "burst should not wait");
        for _ in 0..4 {
            drop(rl.acquire(|| true).unwrap());
        }
        // 4 more requests need ~4 intervals.
        assert!(t.elapsed() >= Duration::from_millis(180), "{:?}", t.elapsed());
    }

    #[test]
    fn caps_in_flight() {
        let rl = RateLimiter::new(Limits { burst: 10.0, ..FAST });
        let a = rl.acquire(|| true).unwrap();
        let _b = rl.acquire(|| true).unwrap();
        let t = Instant::now();
        std::thread::scope(|sc| {
            sc.spawn(|| {
                std::thread::sleep(Duration::from_millis(60));
                drop(a);
            });
            let _c = rl.acquire(|| true).unwrap();
        });
        assert!(t.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn cancels_while_waiting() {
        let rl = RateLimiter::new(Limits { burst: 1.0, interval: Duration::from_secs(60), max_in_flight: 2 });
        drop(rl.acquire(|| true).unwrap());
        let t = Instant::now();
        let deadline = t + Duration::from_millis(150);
        assert!(rl.acquire(|| Instant::now() < deadline).is_err());
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn back_off_blocks() {
        let rl = RateLimiter::new(FAST);
        rl.back_off(Duration::from_millis(120));
        assert!(rl.blocked_for().is_some());
        let t = Instant::now();
        drop(rl.acquire(|| true).unwrap());
        assert!(t.elapsed() >= Duration::from_millis(100));
    }
}
