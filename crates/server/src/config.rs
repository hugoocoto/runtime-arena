//! Server settings, read from the environment.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use crate::net_limit::IpRange;

/// Shortest admin token accepted, so a guessable one is refused at startup.
const MIN_ADMIN_TOKEN_LEN: usize = 24;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    /// Database, problems and submissions live here.
    pub data_dir: PathBuf,
    pub admin_token: String,
    /// Event code that lets people register themselves on `/join`. `None` disables
    /// self-registration, so nobody new can join.
    pub join_code: Option<String>,
    /// One judge worker per isolate box. Box `i` is pinned to the `i`-th CPU of
    /// `JUDGE_CPUS` by the container entrypoint.
    pub judge_boxes: Vec<u32>,
    /// Path of the isolate binary.
    pub isolate: PathBuf,
    /// Times each accepted test runs; the best time and memory count.
    pub runs_per_test: u32,
    /// Processes and threads one submission may create (`ARENA_PROCESSES`).
    pub processes: u32,
    /// Stop judging a submission after this many failing tests; 0 runs all
    /// (`ARENA_MAX_FAILURES`).
    pub max_failures: u32,
    /// Isolate's cgroup root, read for microsecond CPU time (`ARENA_CG_ROOT`).
    pub cg_root: PathBuf,
    /// Root of the size-limited per-box directories (`ARENA_BOX_STORAGE`).
    pub box_storage: Option<PathBuf>,
    /// Periodically ask GitHub for the latest release (`ARENA_UPDATE_CHECK=off` disables).
    pub update_check: bool,
    /// Requests per second allowed from one address (`ARENA_RATE_LIMIT`; 0 disables).
    pub rate_limit: u32,
    /// Reverse proxies whose `X-Forwarded-For` names the client
    /// (`ARENA_TRUSTED_PROXIES`). Empty: the connection's address is the client.
    pub trusted_proxies: Vec<IpRange>,
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());

        let listen = var("ARENA_LISTEN")
            .unwrap_or_else(|| "0.0.0.0:8080".into())
            .parse()
            .map_err(|e| format!("ARENA_LISTEN: {e}"))?;
        let data_dir = var("ARENA_DATA").unwrap_or_else(|| "data".into()).into();
        let admin_token = var("ARENA_ADMIN_TOKEN")
            .ok_or("ARENA_ADMIN_TOKEN is not set; generate one with `openssl rand -hex 32`")?;
        if admin_token.len() < MIN_ADMIN_TOKEN_LEN {
            return Err(format!(
                "ARENA_ADMIN_TOKEN must be at least {MIN_ADMIN_TOKEN_LEN} characters"
            ));
        }
        let join_code = var("ARENA_JOIN_CODE");
        if join_code.as_ref().is_some_and(|c| c.len() < 6) {
            return Err("ARENA_JOIN_CODE must be at least 6 characters".into());
        }
        // The same variable the entrypoint uses to pin boxes; empty means one unpinned worker.
        let workers = var("JUDGE_CPUS")
            .map(|cpus| cpus.split(',').filter(|c| !c.trim().is_empty()).count())
            .unwrap_or(0)
            .max(1);
        let runs_per_test = match var("ARENA_RUNS_PER_TEST") {
            Some(v) => v
                .parse()
                .ok()
                .filter(|n| *n > 0)
                .ok_or("ARENA_RUNS_PER_TEST must be a positive integer")?,
            None => 3,
        };
        let processes = match var("ARENA_PROCESSES") {
            Some(v) => v
                .parse()
                .ok()
                .filter(|n| (1..=4096).contains(n))
                .ok_or("ARENA_PROCESSES must be between 1 and 4096")?,
            None => 64,
        };
        let max_failures = match var("ARENA_MAX_FAILURES") {
            Some(v) => v
                .parse()
                .map_err(|_| "ARENA_MAX_FAILURES must be a whole number (0 runs every test)")?,
            None => 3,
        };
        let cg_root = var("ARENA_CG_ROOT")
            .unwrap_or_else(|| "/sys/fs/cgroup/isolate".into())
            .into();
        let box_storage = Some(
            var("ARENA_BOX_STORAGE")
                .unwrap_or_else(|| "/var/local/lib/arena-boxes".into())
                .into(),
        );
        let update_check = match var("ARENA_UPDATE_CHECK").as_deref() {
            None | Some("on" | "true" | "1") => true,
            Some("off" | "false" | "0") => false,
            Some(_) => return Err("ARENA_UPDATE_CHECK must be on or off".into()),
        };
        let rate_limit = match var("ARENA_RATE_LIMIT") {
            Some(v) => v
                .parse()
                .map_err(|_| "ARENA_RATE_LIMIT must be a whole number (0 disables it)")?,
            None => crate::net_limit::DEFAULT_RATE_LIMIT,
        };
        let trusted_proxies = match var("ARENA_TRUSTED_PROXIES") {
            Some(v) => parse_trusted_proxies(&v, default_gateway)
                .map_err(|e| format!("ARENA_TRUSTED_PROXIES: {e}"))?,
            None => Vec::new(),
        };
        Ok(Config {
            listen,
            data_dir,
            admin_token,
            join_code,
            judge_boxes: (0..workers as u32).collect(),
            isolate: var("ARENA_ISOLATE")
                .unwrap_or_else(|| "isolate".into())
                .into(),
            runs_per_test,
            processes,
            max_failures,
            cg_root,
            box_storage,
            update_check,
            rate_limit,
            trusted_proxies,
        })
    }
}

/// A comma-separated list of addresses and CIDR blocks. `gateway` stands for
/// this machine's default gateway: in a Docker container that is the host, which
/// is where the published port's connections come from.
fn parse_trusted_proxies(
    list: &str,
    gateway: impl Fn() -> Result<IpAddr, String>,
) -> Result<Vec<IpRange>, String> {
    list.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| match s {
            "gateway" => gateway().map(IpRange::single),
            s => IpRange::parse(s),
        })
        .collect()
}

/// The IPv4 default gateway, from `/proc/net/route`.
fn default_gateway() -> Result<IpAddr, String> {
    let table = std::fs::read_to_string("/proc/net/route")
        .map_err(|e| format!("cannot read /proc/net/route for `gateway`: {e}"))?;
    gateway_from_route_table(&table).ok_or_else(|| "no default gateway for `gateway`".into())
}

fn gateway_from_route_table(table: &str) -> Option<IpAddr> {
    table.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // Iface Destination Gateway ...; addresses are hex in host byte order.
        if fields.get(1) != Some(&"00000000") {
            return None;
        }
        let gateway = u32::from_str_radix(fields.get(2)?, 16).ok()?;
        (gateway != 0).then(|| IpAddr::V4(Ipv4Addr::from(gateway.to_le_bytes())))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_proxies_accept_addresses_blocks_and_the_gateway() {
        let gw: IpAddr = "172.23.0.1".parse().unwrap();
        let list = parse_trusted_proxies(" gateway, 10.0.0.0/8 ,", || Ok(gw)).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].contains(gw));
        assert!(list[1].contains("10.1.2.3".parse().unwrap()));
        assert!(parse_trusted_proxies("nonsense", || Ok(gw)).is_err());
        assert!(parse_trusted_proxies("gateway", || Err("none".into())).is_err());
    }

    #[test]
    fn the_gateway_is_read_from_the_route_table() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\n\
                     eth0\t00000000\t010017AC\t0003\t0\t0\t0\t00000000\n\
                     eth0\t000017AC\t00000000\t0001\t0\t0\t0\t0000FFFF\n";
        assert_eq!(
            gateway_from_route_table(table),
            Some("172.23.0.1".parse().unwrap())
        );
        assert_eq!(gateway_from_route_table("Iface Destination\n"), None);
    }
}
