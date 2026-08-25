use dashmap::DashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

pub struct TokenBucket {
    tokens: f64,
    capacity: f64,
    rate_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, rate_per_sec: f64, now: Instant) -> Self {
        TokenBucket {
            tokens: capacity,
            capacity,
            rate_per_sec,
            last: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.capacity);
        self.last = now;
    }

    fn try_take(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

pub struct RateLimiter {
    buckets: DashMap<IpAddr, TokenBucket>,
    capacity: f64,
    rate_per_sec: f64,
    ttl: Duration,
}

impl RateLimiter {
    pub fn new(rate_per_sec: f64, burst: f64) -> Self {
        RateLimiter {
            buckets: DashMap::new(),
            capacity: burst,
            rate_per_sec,
            ttl: Duration::from_secs(600),
        }
    }

    pub fn allow(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        match self.buckets.entry(ip) {
            dashmap::mapref::entry::Entry::Occupied(mut e) => {
                if now.duration_since(e.get().last) > self.ttl {
                    e.remove();
                    let mut b = TokenBucket::new(self.capacity, self.rate_per_sec, now);
                    b.try_take(now)
                } else {
                    e.get_mut().try_take(now)
                }
            }
            dashmap::mapref::entry::Entry::Vacant(v) => {
                let mut b = TokenBucket::new(self.capacity, self.rate_per_sec, now);
                let allowed = b.try_take(now);
                v.insert(b);
                allowed
            }
        }
    }

    pub fn sweep_expired(&self) -> usize {
        let now = Instant::now();
        let mut expired = 0;
        self.buckets.retain(|_, b| {
            if now.duration_since(b.last) > self.ttl {
                expired += 1;
                false
            } else {
                true
            }
        });
        expired
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.buckets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_refill() {
        let rl = RateLimiter::new(10.0, 3.0);
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        assert!(rl.allow(ip));
        assert!(rl.allow(ip));
        assert!(rl.allow(ip));
        assert!(!rl.allow(ip));
    }
}
