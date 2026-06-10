//! Token bucket rate limiter — prevents agent loops from exhausting system resources.

use std::time::Instant;

pub struct RateLimiter {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,
    last_refill: Instant,
    enabled: bool,
}

impl RateLimiter {
    pub fn new(max_tokens: f64, refill_rate: f64) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            refill_rate,
            last_refill: Instant::now(),
            enabled: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            tokens: 0.0,
            max_tokens: 0.0,
            refill_rate: 0.0,
            last_refill: Instant::now(),
            enabled: false,
        }
    }

    #[allow(dead_code)]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn try_acquire(&mut self) -> bool {
        if !self.enabled {
            return true;
        }
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }

    #[cfg(test)]
    fn token_count(&self) -> f64 {
        self.tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_within_burst() {
        let mut limiter = RateLimiter::new(5.0, 1.0);
        for _ in 0..5 {
            assert!(limiter.try_acquire());
        }
        assert!(!limiter.try_acquire());
    }

    #[test]
    fn refill_after_time() {
        let mut limiter = RateLimiter::new(2.0, 100.0);
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(!limiter.try_acquire());
        limiter.last_refill = Instant::now() - std::time::Duration::from_millis(50);
        assert!(limiter.try_acquire());
    }

    #[test]
    fn disabled_limiter_always_allows() {
        let mut limiter = RateLimiter::disabled();
        for _ in 0..1000 {
            assert!(limiter.try_acquire());
        }
    }

    #[test]
    fn disabled_reports_enabled_false() {
        let limiter = RateLimiter::disabled();
        assert!(!limiter.enabled());
        let limiter = RateLimiter::new(10.0, 5.0);
        assert!(limiter.enabled());
    }

    #[test]
    fn tokens_never_exceed_max() {
        let mut limiter = RateLimiter::new(3.0, 100.0);
        limiter.last_refill = Instant::now() - std::time::Duration::from_secs(100);
        limiter.refill();
        assert!(limiter.token_count() <= 3.0);
    }
}
