//! Limits on what one network address can use: requests per second and open
//! connections (WebSockets included). Both are far above what clients need, since
//! a whole contest room may share one address. They are kept in memory.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::connect_info::Connected;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::http::header::RETRY_AFTER;
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

/// Refuses requests over the rate. `/healthz` is exempt, so a flood can't make
/// Docker restart a working server.
pub async fn limit_requests(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<Peer>>()
        .map(|c| c.0.0.ip());
    if let Some(ip) = peer
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

#[derive(Debug, Default)]
struct Open {
    per_address: HashMap<IpAddr, u32>,
    total: u32,
}

/// Counts open connections.
#[derive(Debug)]
struct ConnectionLimiter {
    open: Mutex<Open>,
    per_address: u32,
    total: u32,
}

/// One open connection; dropping it frees the slot.
#[derive(Debug)]
struct ConnectionSlot {
    limiter: Arc<ConnectionLimiter>,
    ip: IpAddr,
}

impl ConnectionLimiter {
    fn new(per_address: u32, total: u32) -> Arc<Self> {
        Arc::new(ConnectionLimiter {
            open: Mutex::default(),
            per_address,
            total,
        })
    }

    fn acquire(self: &Arc<Self>, ip: IpAddr) -> Option<ConnectionSlot> {
        let ip = normalise(ip);
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.total >= self.total {
            return None;
        }
        let count = open.per_address.entry(ip).or_insert(0);
        if *count >= self.per_address {
            return None;
        }
        *count += 1;
        open.total += 1;
        Some(ConnectionSlot {
            limiter: self.clone(),
            ip,
        })
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        let mut open = self.limiter.open.lock().unwrap_or_else(|e| e.into_inner());
        open.total = open.total.saturating_sub(1);
        if let Some(count) = open.per_address.get_mut(&self.ip) {
            *count -= 1;
            if *count == 0 {
                open.per_address.remove(&self.ip);
            }
        }
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
pub struct LimitedListener {
    inner: TcpListener,
    limiter: Arc<ConnectionLimiter>,
}

impl LimitedListener {
    pub fn new(inner: TcpListener) -> Self {
        LimitedListener {
            inner,
            limiter: ConnectionLimiter::new(MAX_CONNECTIONS_PER_ADDRESS, MAX_CONNECTIONS),
        }
    }
}

impl Listener for LimitedListener {
    type Io = LimitedStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, addr) = Listener::accept(&mut self.inner).await;
            if let Some(slot) = self.limiter.acquire(addr.ip()) {
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
    _slot: ConnectionSlot,
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
        let l = ConnectionLimiter::new(2, 3);
        let a = l.acquire(ip(1)).unwrap();
        let _b = l.acquire(ip(1)).unwrap();
        assert!(l.acquire(ip(1)).is_none());
        let _c = l.acquire(ip(2)).unwrap();
        assert!(l.acquire(ip(3)).is_none(), "the total is full");
        drop(a);
        assert!(l.acquire(ip(1)).is_some());
    }
}
