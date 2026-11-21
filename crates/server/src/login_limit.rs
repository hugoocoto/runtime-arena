//! Limits guessing of passwords and of the event code, in memory (a restart
//! forgets it).
//!
//! Attempts are counted per account *and* address, so a stranger can't lock a
//! participant out by failing on purpose. A looser limit per address stops one
//! machine from trying a few passwords on every account. It stays high because
//! a whole contest room may share one address. Wrong event codes use the same
//! per-address budget: the code is shared, so there is no account to count by.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Failed logins allowed for one account from one address, per window.
const MAX_PER_ACCOUNT: u32 = 10;
/// Failed logins and wrong event codes allowed from one address, per window.
const MAX_PER_ADDRESS: u32 = 100;
/// Accounts one address may try to create, per window.
const MAX_SIGNUPS_PER_ADDRESS: u32 = 300;
const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Bounds memory if many addresses attack at once.
const MAX_ENTRIES: usize = 100_000;
const PRUNE_EVERY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Address(IpAddr),
    Account(String, IpAddr),
    Signup(IpAddr),
}

#[derive(Debug)]
struct Entry {
    count: u32,
    resets_at: Instant,
}

#[derive(Debug, Default)]
struct Entries {
    map: HashMap<Key, Entry>,
    last_prune: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct LoginLimiter {
    entries: Mutex<Entries>,
}

/// One IPv6 host usually controls a whole /64, so it counts as one address.
pub(crate) fn normalise(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let s = v6.segments();
            IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
        v4 => v4,
    }
}

impl Entries {
    /// Counts an attempt, or says how long until one is allowed.
    fn reserve(&mut self, key: Key, limit: u32, now: Instant) -> Result<(), Duration> {
        if self.map.len() >= MAX_ENTRIES
            && self
                .last_prune
                .is_none_or(|t| now.duration_since(t) >= PRUNE_EVERY)
        {
            self.map.retain(|_, e| e.resets_at > now);
            self.last_prune = Some(now);
        }
        if let Some(e) = self.map.get_mut(&key) {
            if e.resets_at > now {
                if e.count >= limit {
                    return Err(e.resets_at - now);
                }
                e.count += 1;
            } else {
                *e = Entry {
                    count: 1,
                    resets_at: now + WINDOW,
                };
            }
            return Ok(());
        }
        if self.map.len() >= MAX_ENTRIES {
            return Err(PRUNE_EVERY);
        }
        self.map.insert(
            key,
            Entry {
                count: 1,
                resets_at: now + WINDOW,
            },
        );
        Ok(())
    }

    /// Takes back one counted attempt.
    fn release(&mut self, key: &Key) {
        if let Some(e) = self.map.get_mut(key) {
            e.count = e.count.saturating_sub(1);
            if e.count == 0 {
                self.map.remove(key);
            }
        }
    }
}

impl LoginLimiter {
    /// Call before checking a password, so parallel guesses are counted too.
    pub fn begin(&self, name: &str, ip: IpAddr) -> Result<(), Duration> {
        self.begin_at(name, ip, Instant::now())
    }

    fn begin_at(&self, name: &str, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let ip = normalise(ip);
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let address = Key::Address(ip);
        entries.reserve(address.clone(), MAX_PER_ADDRESS, now)?;
        let account = Key::Account(name.to_owned(), ip);
        let result = entries.reserve(account, MAX_PER_ACCOUNT, now);
        if result.is_err() {
            // Refused without trying a password, so it doesn't count.
            entries.release(&address);
        }
        result
    }

    /// Call before checking an event code.
    pub fn begin_code(&self, ip: IpAddr) -> Result<(), Duration> {
        self.begin_code_at(ip, Instant::now())
    }

    fn begin_code_at(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.reserve(Key::Address(normalise(ip)), MAX_PER_ADDRESS, now)
    }

    /// A correct event code: it doesn't count as a failure.
    pub fn code_accepted(&self, ip: IpAddr) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.release(&Key::Address(normalise(ip)));
    }

    /// Call before creating an account, whose password hash is costly.
    pub fn begin_signup(&self, ip: IpAddr) -> Result<(), Duration> {
        self.begin_signup_at(ip, Instant::now())
    }

    fn begin_signup_at(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.reserve(Key::Signup(normalise(ip)), MAX_SIGNUPS_PER_ADDRESS, now)
    }

    /// A correct password: forgets this account's failures from this address.
    pub fn succeeded(&self, name: &str, ip: IpAddr) {
        let ip = normalise(ip);
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.release(&Key::Address(ip));
        entries.map.remove(&Key::Account(name.to_owned(), ip));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, last])
    }

    #[test]
    fn an_account_is_limited_per_address() {
        let l = LoginLimiter::default();
        let now = Instant::now();
        for _ in 0..MAX_PER_ACCOUNT {
            l.begin_at("dana", ip(1), now).unwrap();
        }
        assert!(l.begin_at("dana", ip(1), now).is_err());
        // The real owner, elsewhere, and other accounts are not locked out.
        assert!(l.begin_at("dana", ip(2), now).is_ok());
        assert!(l.begin_at("erin", ip(1), now).is_ok());
    }

    #[test]
    fn the_limit_ends_with_the_window() {
        let l = LoginLimiter::default();
        let now = Instant::now();
        for _ in 0..MAX_PER_ACCOUNT {
            l.begin_at("dana", ip(1), now).unwrap();
        }
        let wait = l.begin_at("dana", ip(1), now).unwrap_err();
        assert_eq!(wait, WINDOW);
        assert!(l.begin_at("dana", ip(1), now + WINDOW).is_ok());
    }

    #[test]
    fn one_address_cannot_spray_every_account() {
        let l = LoginLimiter::default();
        let now = Instant::now();
        for i in 0..MAX_PER_ADDRESS {
            l.begin_at(&format!("user{i}"), ip(1), now).unwrap();
        }
        assert!(l.begin_at("another", ip(1), now).is_err());
        assert!(l.begin_at("another", ip(2), now).is_ok());
    }

    #[test]
    fn hammering_a_locked_account_uses_no_address_budget() {
        let l = LoginLimiter::default();
        let now = Instant::now();
        for _ in 0..MAX_PER_ACCOUNT + 500 {
            let _ = l.begin_at("dana", ip(1), now);
        }
        assert!(l.begin_at("erin", ip(1), now).is_ok());
    }

    #[test]
    fn success_clears_only_that_account() {
        let l = LoginLimiter::default();
        let now = Instant::now();
        for _ in 0..MAX_PER_ACCOUNT {
            l.begin_at("dana", ip(1), now).unwrap();
            l.begin_at("erin", ip(1), now).unwrap();
        }
        l.succeeded("dana", ip(1));
        assert!(l.begin_at("dana", ip(1), now).is_ok());
        assert!(l.begin_at("erin", ip(1), now).is_err());
    }

    #[test]
    fn wrong_event_codes_share_the_address_budget() {
        let l = LoginLimiter::default();
        let now = Instant::now();
        for _ in 0..MAX_PER_ADDRESS {
            l.begin_code_at(ip(1), now).unwrap();
        }
        assert!(l.begin_code_at(ip(1), now).is_err());
        assert!(l.begin_at("dana", ip(1), now).is_err());
        assert!(l.begin_code_at(ip(2), now).is_ok());
    }

    #[test]
    fn correct_event_codes_are_not_counted() {
        let l = LoginLimiter::default();
        let now = Instant::now();
        for _ in 0..MAX_PER_ADDRESS * 2 {
            l.begin_code_at(ip(1), now).unwrap();
            l.code_accepted(ip(1));
        }
        assert!(l.begin_code_at(ip(1), now).is_ok());
    }

    #[test]
    fn account_creation_is_limited_per_address() {
        let l = LoginLimiter::default();
        let now = Instant::now();
        for _ in 0..MAX_SIGNUPS_PER_ADDRESS {
            l.begin_signup_at(ip(1), now).unwrap();
        }
        assert!(l.begin_signup_at(ip(1), now).is_err());
        assert!(l.begin_signup_at(ip(2), now).is_ok());
        assert!(
            l.begin_at("dana", ip(1), now).is_ok(),
            "logins are separate"
        );
    }

    #[test]
    fn an_ipv6_host_cannot_rotate_within_its_subnet() {
        let a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        assert_eq!(normalise(a), normalise(b));
        let mapped: IpAddr = "::ffff:10.0.0.1".parse().unwrap();
        assert_eq!(normalise(mapped), ip(1));
    }
}
