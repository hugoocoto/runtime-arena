//! Limits on what one network address can use: requests per second, open
//! connections and open WebSockets. They are far above what clients need, since
//! a whole contest room may share one address. They are kept in memory.
//!
//! Behind a reverse proxy (Caddy in `compose.yaml`) every connection comes from
//! the proxy, so the client's address is taken from `X-Forwarded-For`, but only
//! when the connection comes from a proxy listed in `ARENA_TRUSTED_PROXIES`.

use std::collections::HashMap;
use std::hash::Hash;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::serve::{IncomingStream, Listener};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};

use crate::AppState;
use crate::login_limit::normalise;

/// Requests per second from one address, unless `ARENA_RATE_LIMIT` says otherwise.
/// A client needs about one; this leaves room for 200 behind one address.
pub const DEFAULT_RATE_LIMIT: u32 = 200;
/// A short burst may reach this many seconds' worth of requests.
const BURST_SECONDS: u32 = 5;
const MAX_TRACKED: usize = 100_000;
const PRUNE_EVERY: Duration = Duration::from_secs(60);

const MAX_CONNECTIONS_PER_ADDRESS: u32 = 512;
const MAX_CONNECTIONS: u32 = 4096;

/// WebSockets stay open for the whole contest, so they get a budget of their own
/// below [`MAX_CONNECTIONS`]: a flood of them can't take the slots that logins
/// and submissions need.
const MAX_WEBSOCKETS: u32 = 3072;
const _: () = assert!(MAX_WEBSOCKETS < MAX_CONNECTIONS);
/// Anonymous WebSockets (projectors, the leaderboard page) from one address.
const MAX_WEBSOCKETS_PER_ADDRESS: u32 = 256;
/// WebSockets of one logged-in participant, whatever their address.
const MAX_WEBSOCKETS_PER_USER: u32 = 16;

/// An address or a CIDR block, from `ARENA_TRUSTED_PROXIES`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpRange {
    net: IpAddr,
    prefix: u8,
}

impl IpRange {
    /// `10.0.0.1`, `172.16.0.0/12` or `fd00::/8`.
    pub fn parse(s: &str) -> Result<IpRange, String> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (s, None),
        };
        let net: IpAddr = addr
            .parse()
            .map_err(|_| format!("{s:?} is not an IP address or CIDR block"))?;
        let net = net.to_canonical();
        let max = if net.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p
                .parse()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("{s:?} has an invalid prefix length"))?,
            None => max,
        };
        Ok(IpRange { net, prefix })
    }

    pub fn single(ip: IpAddr) -> IpRange {
        let net = ip.to_canonical();
        IpRange {
            net,
            prefix: if net.is_ipv4() { 32 } else { 128 },
        }
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.net, ip.to_canonical()) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

fn is_trusted(trusted: &[IpRange], ip: IpAddr) -> bool {
    trusted.iter().any(|r| r.contains(ip))
}

/// The client's address: the connection's other end, or, when that is a
/// trusted proxy, the address the proxy reports. The proxy appends the address
/// it saw to `X-Forwarded-For`, so the list is read from the right, skipping
/// trusted proxies; entries further left were written by the client and could
/// be anything.
pub fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[IpRange]) -> IpAddr {
    if !is_trusted(trusted, peer) {
        return peer;
    }
    let entries: Vec<&str> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .collect();
    let mut client = peer;
    for entry in entries.iter().rev() {
        let Ok(ip) = entry.trim().parse::<IpAddr>() else {
            break;
        };
        client = ip;
        if !is_trusted(trusted, ip) {
            break;
        }
    }
    client
}

/// The client's address for this request, resolved once by [`limit_requests`].
#[derive(Debug, Clone, Copy)]
pub struct ClientIp(pub IpAddr);

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

#[derive(Debug, Default)]
struct Buckets {
    map: HashMap<IpAddr, Bucket>,
    last_prune: Option<Instant>,
}

/// A token bucket per address. A rate of 0 turns it off.
#[derive(Debug)]
pub struct RequestLimiter {
    rate: u32,
    buckets: Mutex<Buckets>,
}

impl RequestLimiter {
    pub fn new(rate: u32) -> Self {
        RequestLimiter {
            rate,
            buckets: Mutex::default(),
        }
    }

    /// Takes one token, or says how long until one is available.
    fn check(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        if self.rate == 0 {
            return Ok(());
        }
        let rate = f64::from(self.rate);
        let burst = rate * f64::from(BURST_SECONDS);
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if buckets.map.len() >= MAX_TRACKED
            && buckets
                .last_prune
                .is_none_or(|t| now.duration_since(t) >= PRUNE_EVERY)
        {
            // A bucket that has refilled is the same as no bucket.
            let full_after = Duration::from_secs(u64::from(BURST_SECONDS));
            buckets
                .map
                .retain(|_, b| now.duration_since(b.updated) < full_after);
            buckets.last_prune = Some(now);
        }
        let ip = normalise(ip);
        if !buckets.map.contains_key(&ip) && buckets.map.len() >= MAX_TRACKED {
            // Refusing here would let a huge botnet block everyone else.
            return Ok(());
        }
        let bucket = buckets.map.entry(ip).or_insert(Bucket {
            tokens: burst,
            updated: now,
        });
        let elapsed = now.duration_since(bucket.updated).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * rate).min(burst);
        bucket.updated = now;
        if bucket.tokens < 1.0 {
            return Err(Duration::from_secs_f64((1.0 - bucket.tokens) / rate));
        }
        bucket.tokens -= 1.0;
        Ok(())
    }
}

/// Works out the client's address ([`ClientIp`]) and refuses requests over the
/// rate. `/healthz` is exempt, so a flood can't make Docker restart a working
/// server.
pub async fn limit_requests(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<Peer>>()
        .map(|c| c.0.0.ip());
    let client = peer.map(|p| client_ip(p, req.headers(), &state.config.trusted_proxies));
    if let Some(ip) = client {
        req.extensions_mut().insert(ClientIp(ip));
    }
    if let Some(ip) = client
        && req.uri().path() != "/healthz"
        && let Err(wait) = state.request_limit.check(ip, Instant::now())
    {
        let body = arena_protocol::ApiError {
            error: "too many requests; slow down and try again".to_owned(),
        };
        let seconds = wait.as_secs().saturating_add(1).to_string();
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(RETRY_AFTER, seconds)],
            Json(body),
        )
            .into_response();
    }
    next.run(req).await
}

#[derive(Debug)]
struct Open<K> {
    per_key: HashMap<K, u32>,
    total: u32,
}

/// Counts open connections, in total and per key (an address, or a participant).
#[derive(Debug)]
struct SlotLimiter<K: Hash + Eq + Clone> {
    open: Mutex<Open<K>>,
    total: u32,
}

/// One open connection; dropping it frees the slot.
#[derive(Debug)]
struct Slot<K: Hash + Eq + Clone> {
    limiter: Arc<SlotLimiter<K>>,
    /// `None` when only the total counts it.
    key: Option<K>,
}

impl<K: Hash + Eq + Clone> SlotLimiter<K> {
    fn new(total: u32) -> Arc<Self> {
        Arc::new(SlotLimiter {
            open: Mutex::new(Open {
                per_key: HashMap::new(),
                total: 0,
            }),
            total,
        })
    }

    /// Takes a slot, if the total and the key's count (up to `limit`) allow it.
    fn acquire(self: &Arc<Self>, key: Option<(K, u32)>) -> Option<Slot<K>> {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.total >= self.total {
            return None;
        }
        let key = match key {
            Some((key, limit)) => {
                let count = open.per_key.entry(key.clone()).or_insert(0);
                if *count >= limit {
                    return None;
                }
                *count += 1;
                Some(key)
            }
            None => None,
        };
        open.total += 1;
        Some(Slot {
            limiter: self.clone(),
            key,
        })
    }
}

impl<K: Hash + Eq + Clone> Drop for Slot<K> {
    fn drop(&mut self) {
        let mut open = self.limiter.open.lock().unwrap_or_else(|e| e.into_inner());
        open.total = open.total.saturating_sub(1);
        if let Some(key) = &self.key
            && let Some(count) = open.per_key.get_mut(key)
        {
            *count -= 1;
            if *count == 0 {
                open.per_key.remove(key);
            }
        }
    }
}

/// Who a WebSocket counts against.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SocketOwner {
    Address(IpAddr),
    User(i64),
}

/// Open WebSockets: a share of the connection budget, per participant or, for
/// anonymous viewers, per address.
#[derive(Debug)]
pub struct WebSocketLimiter {
    slots: Arc<SlotLimiter<SocketOwner>>,
}

/// One open WebSocket; dropping it frees the slot.
#[derive(Debug)]
pub struct WebSocketSlot {
    _slot: Slot<SocketOwner>,
}

impl Default for WebSocketLimiter {
    fn default() -> Self {
        WebSocketLimiter {
            slots: SlotLimiter::new(MAX_WEBSOCKETS),
        }
    }
}

impl WebSocketLimiter {
    pub fn acquire(&self, user: Option<i64>, ip: IpAddr) -> Option<WebSocketSlot> {
        let key = match user {
            Some(id) => (SocketOwner::User(id), MAX_WEBSOCKETS_PER_USER),
            None => (
                SocketOwner::Address(normalise(ip)),
                MAX_WEBSOCKETS_PER_ADDRESS,
            ),
        };
        self.slots
            .acquire(Some(key))
            .map(|slot| WebSocketSlot { _slot: slot })
    }
}

/// The address of the connection's other end.
#[derive(Debug, Clone, Copy)]
pub struct Peer(pub SocketAddr);

impl Connected<IncomingStream<'_, LimitedListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, LimitedListener>) -> Self {
        Peer(*stream.remote_addr())
    }
}

/// A TCP listener that closes connections over the limits right away.
/// Connections from a trusted proxy count only towards the total: they carry
/// many clients, whose own limits apply per request.
pub struct LimitedListener {
    inner: TcpListener,
    limiter: Arc<SlotLimiter<IpAddr>>,
    trusted: Vec<IpRange>,
}

impl LimitedListener {
    pub fn new(inner: TcpListener, trusted: Vec<IpRange>) -> Self {
        LimitedListener {
            inner,
            limiter: SlotLimiter::new(MAX_CONNECTIONS),
            trusted,
        }
    }

    fn acquire(&self, ip: IpAddr) -> Option<Slot<IpAddr>> {
        let key =
            (!is_trusted(&self.trusted, ip)).then(|| (normalise(ip), MAX_CONNECTIONS_PER_ADDRESS));
        self.limiter.acquire(key)
    }
}

impl Listener for LimitedListener {
    type Io = LimitedStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, addr) = Listener::accept(&mut self.inner).await;
            if let Some(slot) = self.acquire(addr.ip()) {
                return (
                    LimitedStream {
                        stream,
                        _slot: slot,
                    },
                    addr,
                );
            }
            tracing::debug!(peer = %addr.ip(), "connection refused: too many open");
        }
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        Listener::local_addr(&self.inner)
    }
}

/// A TCP stream holding its connection slot.
pub struct LimitedStream {
    stream: TcpStream,
    _slot: Slot<IpAddr>,
}

impl AsyncRead for LimitedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for LimitedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, last])
    }

    #[test]
    fn requests_are_limited_per_address_and_refill() {
        let l = RequestLimiter::new(2);
        let now = Instant::now();
        for _ in 0..2 * BURST_SECONDS {
            l.check(ip(1), now).unwrap();
        }
        assert!(l.check(ip(1), now).is_err());
        assert!(l.check(ip(2), now).is_ok());
        assert!(l.check(ip(1), now + Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn a_rate_of_zero_is_off() {
        let l = RequestLimiter::new(0);
        let now = Instant::now();
        for _ in 0..10_000 {
            l.check(ip(1), now).unwrap();
        }
    }

    #[test]
    fn connections_are_limited_and_slots_come_back() {
        let l = SlotLimiter::new(3);
        let a = l.acquire(Some((ip(1), 2))).unwrap();
        let _b = l.acquire(Some((ip(1), 2))).unwrap();
        assert!(l.acquire(Some((ip(1), 2))).is_none());
        let _c = l.acquire(Some((ip(2), 2))).unwrap();
        assert!(l.acquire(Some((ip(3), 2))).is_none(), "the total is full");
        drop(a);
        assert!(l.acquire(Some((ip(1), 2))).is_some());
    }

    #[test]
    fn unkeyed_slots_count_only_towards_the_total() {
        let l = SlotLimiter::new(3);
        let _a = l.acquire(None).unwrap();
        let _b = l.acquire(None).unwrap();
        let _c = l.acquire(Some((ip(1), 1))).unwrap();
        assert!(l.acquire(None).is_none());
    }

    #[test]
    fn websockets_leave_room_for_other_connections() {
        let l = WebSocketLimiter::default();
        let held: Vec<_> = (0..MAX_WEBSOCKETS_PER_ADDRESS)
            .map(|_| l.acquire(None, ip(1)).unwrap())
            .collect();
        assert!(l.acquire(None, ip(1)).is_none(), "anonymous, per address");
        assert!(
            l.acquire(Some(7), ip(1)).is_some(),
            "participants count apart"
        );
        let mine: Vec<_> = (0..MAX_WEBSOCKETS_PER_USER)
            .map(|_| l.acquire(Some(7), ip(2)).unwrap())
            .collect();
        assert!(l.acquire(Some(7), ip(3)).is_none(), "per participant");
        drop((held, mine));
    }

    #[test]
    fn ip_ranges_match_addresses_and_blocks() {
        let r = IpRange::parse("172.16.0.0/12").unwrap();
        assert!(r.contains("172.23.0.1".parse().unwrap()));
        assert!(r.contains("::ffff:172.31.255.255".parse().unwrap()));
        assert!(!r.contains("172.32.0.1".parse().unwrap()));
        assert!(!r.contains("fd00::1".parse().unwrap()));
        let one = IpRange::parse("10.0.0.1").unwrap();
        assert!(one.contains(ip(1)) && !one.contains(ip(2)));
        assert!(IpRange::parse("0.0.0.0/0").unwrap().contains(ip(9)));
        assert!(
            IpRange::parse("fd00::/8")
                .unwrap()
                .contains("fd12::3".parse().unwrap())
        );
        assert!(IpRange::parse("10.0.0.0/33").is_err());
        assert!(IpRange::parse("example.com").is_err());
    }

    fn forwarded(values: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append("x-forwarded-for", v.parse().unwrap());
        }
        h
    }

    #[test]
    fn forwarded_for_is_read_only_from_trusted_proxies() {
        let proxy: IpAddr = "172.23.0.1".parse().unwrap();
        let trusted = [IpRange::single(proxy)];
        let h = forwarded(&["198.51.100.7"]);
        assert_eq!(
            client_ip(proxy, &h, &trusted),
            "198.51.100.7".parse::<IpAddr>().unwrap()
        );
        assert_eq!(client_ip(ip(1), &h, &trusted), ip(1), "not a proxy");
        assert_eq!(client_ip(proxy, &h, &[]), proxy, "nothing trusted");
        assert_eq!(client_ip(proxy, &HeaderMap::new(), &trusted), proxy);
        assert_eq!(client_ip(proxy, &forwarded(&["junk"]), &trusted), proxy);
    }

    #[test]
    fn a_client_cannot_choose_its_forwarded_address() {
        let proxy: IpAddr = "172.23.0.1".parse().unwrap();
        let trusted = [IpRange::single(proxy)];
        // The client sent "1.2.3.4"; the proxy appended the address it saw.
        let real: IpAddr = "198.51.100.7".parse().unwrap();
        assert_eq!(
            client_ip(proxy, &forwarded(&["1.2.3.4, 198.51.100.7"]), &trusted),
            real
        );
        assert_eq!(
            client_ip(proxy, &forwarded(&["1.2.3.4", "198.51.100.7"]), &trusted),
            real
        );
        // A chain of trusted proxies is skipped.
        let two = [
            IpRange::single(proxy),
            IpRange::parse("10.9.0.0/16").unwrap(),
        ];
        assert_eq!(
            client_ip(
                proxy,
                &forwarded(&["1.2.3.4, 198.51.100.7, 10.9.1.1"]),
                &two
            ),
            real
        );
    }
}
