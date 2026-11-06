//! Per-host adaptive concurrency (AIMD) for document downloads.
//!
//! Replaces the old single user-chosen "parallel downloads" number. Every
//! host starts with a small window that grows by ~1 slot per window of
//! successful transfers and halves (plus a cool-down) the moment the host
//! answers 429/503. The result: fast, healthy hosts (CDNs, big repositories)
//! ramp up quickly while busy ones are backed off automatically — no setting.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// Ceiling per host. A few repositories publish (or are known for) stricter
/// limits for automated clients.
fn host_max(host: &str) -> f32 {
    let h = host.trim_start_matches("www.");
    match h {
        "arxiv.org" | "export.arxiv.org" => 2.0,
        "zenodo.org" => 3.0,
        "api.semanticscholar.org" | "www.semanticscholar.org" | "semanticscholar.org" => 2.0,
        "archive.org" | "ia800000.us.archive.org" => 4.0,
        "www.gutenberg.org" | "gutenberg.org" => 3.0,
        _ if h.ends_with(".archive.org") => 4.0,
        _ => 6.0,
    }
}

const START: f32 = 2.0;
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(5);
const MAX_COOLDOWN: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct HostState {
    limit: f32,
    max: f32,
    in_flight: usize,
    cooldown_until: Option<Instant>,
    /// Successful document downloads from this host (this run).
    ok: u32,
    /// Refusals: 401/403, or "a web page, not a document".
    refusals: u32,
}

impl HostState {
    fn new(host: &str) -> Self {
        let max = host_max(host);
        HostState {
            limit: START.min(max),
            max,
            in_flight: 0,
            cooldown_until: None,
            ok: 0,
            refusals: 0,
        }
    }
}

/// Refusals (with no success) after which a host is skipped for the rest of
/// the run, so documents whose only copy lives there are dropped quietly
/// instead of failing one by one.
const BLOCK_AFTER: u32 = 2;

#[derive(Debug, Default)]
pub struct HostLimiter {
    hosts: Mutex<HashMap<String, HostState>>,
    /// Hosts known (from earlier runs) to refuse automated downloads.
    preblocked: Mutex<std::collections::HashSet<String>>,
    notify: Notify,
}

pub fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default()
}

impl HostLimiter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Wait for a slot on `host`. Returns `None` if `cancel` fires first.
    pub async fn acquire(
        self: &Arc<Self>,
        host: &str,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Option<HostPermit> {
        loop {
            let wait = {
                let mut map = self.hosts.lock();
                let st = map
                    .entry(host.to_string())
                    .or_insert_with(|| HostState::new(host));
                let now = Instant::now();
                match st.cooldown_until {
                    Some(t) if t > now => Some(t - now),
                    _ => {
                        st.cooldown_until = None;
                        if (st.in_flight as f32) < st.limit.floor().max(1.0) {
                            st.in_flight += 1;
                            return Some(HostPermit {
                                limiter: Arc::clone(self),
                                host: host.to_string(),
                            });
                        }
                        None
                    }
                }
            };
            let nap = wait.unwrap_or(Duration::from_millis(500));
            tokio::select! {
                _ = cancel.cancelled() => return None,
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(nap) => {}
            }
        }
    }

    /// Success on `host`: additive increase (≈ +1 per window of successes).
    pub fn reward(&self, host: &str) {
        if let Some(st) = self.hosts.lock().get_mut(host) {
            st.limit = (st.limit + 1.0 / st.limit.max(1.0)).min(st.max);
        }
        self.notify.notify_waiters();
    }

    /// The host asked us to slow down: halve the window and pause it.
    pub fn penalize(&self, host: &str, retry_after: Option<Duration>) {
        if let Some(st) = self.hosts.lock().get_mut(host) {
            st.limit = (st.limit / 2.0).max(1.0);
            let pause = retry_after.unwrap_or(DEFAULT_COOLDOWN).min(MAX_COOLDOWN);
            st.cooldown_until = Some(Instant::now() + pause);
        }
    }

    /// A document downloaded successfully from `host`.
    pub fn record_ok(&self, host: &str) {
        let mut map = self.hosts.lock();
        map.entry(host.to_string())
            .or_insert_with(|| HostState::new(host))
            .ok += 1;
        drop(map);
        self.preblocked.lock().remove(host);
    }

    /// `host` refused a document (401/403, or served a page with no file).
    pub fn record_refusal(&self, host: &str) {
        self.hosts
            .lock()
            .entry(host.to_string())
            .or_insert_with(|| HostState::new(host))
            .refusals += 1;
    }

    /// True if downloads from `host` should not be attempted.
    pub fn is_blocked(&self, host: &str) -> bool {
        if host.is_empty() {
            return false;
        }
        let map = self.hosts.lock();
        match map.get(host) {
            Some(st) if st.ok > 0 => false,
            Some(st) if st.refusals >= BLOCK_AFTER => true,
            _ => {
                drop(map);
                self.preblocked.lock().contains(host)
            }
        }
    }

    /// Seed hosts that refused downloads in earlier runs.
    pub fn preblock<I: IntoIterator<Item = String>>(&self, hosts: I) {
        self.preblocked.lock().extend(hosts);
    }

    /// `(blocked, good)` hosts observed this run, for persisting.
    pub fn reputation(&self) -> (Vec<String>, Vec<String>) {
        let map = self.hosts.lock();
        let mut blocked = Vec::new();
        let mut good = Vec::new();
        for (h, st) in map.iter() {
            if st.ok > 0 {
                good.push(h.clone());
            } else if st.refusals >= BLOCK_AFTER {
                blocked.push(h.clone());
            }
        }
        (blocked, good)
    }

    #[cfg(test)]
    fn limit_of(&self, host: &str) -> f32 {
        self.hosts.lock().get(host).map(|s| s.limit).unwrap_or(0.0)
    }

    fn release(&self, host: &str) {
        if let Some(st) = self.hosts.lock().get_mut(host) {
            st.in_flight = st.in_flight.saturating_sub(1);
        }
        self.notify.notify_waiters();
    }
}

/// RAII slot on a host; released on drop.
pub struct HostPermit {
    limiter: Arc<HostLimiter>,
    host: String,
}

impl HostPermit {
    pub fn host(&self) -> &str {
        &self.host
    }
}

impl Drop for HostPermit {
    fn drop(&mut self) {
        self.limiter.release(&self.host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn grows_on_success_and_halves_on_throttle() {
        let l = HostLimiter::new();
        let c = CancellationToken::new();
        let p = l.acquire("example.org", &c).await.unwrap();
        assert_eq!(l.limit_of("example.org"), 2.0);
        for _ in 0..10 {
            l.reward("example.org");
        }
        assert!(l.limit_of("example.org") > 4.0);
        assert!(l.limit_of("example.org") <= 6.0);
        l.penalize("example.org", Some(Duration::from_millis(1)));
        assert!(l.limit_of("example.org") <= 3.0);
        drop(p);
    }

    #[tokio::test]
    async fn respects_window_and_cancel() {
        let l = HostLimiter::new();
        let c = CancellationToken::new();
        let _a = l.acquire("arxiv.org", &c).await.unwrap();
        let _b = l.acquire("arxiv.org", &c).await.unwrap();
        // Window is 2 for arXiv; a third acquire must wait until cancelled.
        let l2 = Arc::clone(&l);
        let c2 = c.clone();
        let h = tokio::spawn(async move { l2.acquire("arxiv.org", &c2).await.is_none() });
        tokio::time::sleep(Duration::from_millis(50)).await;
        c.cancel();
        assert!(h.await.unwrap());
    }

    #[test]
    fn hosts_that_refuse_get_blocked_until_they_succeed() {
        let l = HostLimiter::new();
        assert!(!l.is_blocked("pub.example"));
        l.record_refusal("pub.example");
        assert!(!l.is_blocked("pub.example"));
        l.record_refusal("pub.example");
        assert!(l.is_blocked("pub.example"));
        l.record_ok("pub.example");
        assert!(!l.is_blocked("pub.example"));
        l.preblock(["old.example".to_string()]);
        assert!(l.is_blocked("old.example"));
        l.record_ok("old.example");
        assert!(!l.is_blocked("old.example"));
        let (blocked, good) = l.reputation();
        assert!(blocked.is_empty());
        assert_eq!(good.len(), 2);
    }

    #[test]
    fn host_parsing() {
        assert_eq!(host_of("https://ArXiv.org/pdf/1"), "arxiv.org");
        assert_eq!(host_of("nonsense"), "");
    }
}
