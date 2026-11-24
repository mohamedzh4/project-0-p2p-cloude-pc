//! DNS that keeps working when the PC's own DNS does not.
//!
//! A real host showed `Dns Failed: resolve dns name 'test-projet0.duckdns.org:443'
//! (os error 11001)`: the PC could not resolve the server name, so heartbeats and
//! command polls stopped and the Matchmaker dropped the host. The cause is the
//! PC's resolver or its ISP, not the server.
//!
//! This resolver only decides **which IP addresses to try**. TLS still checks the
//! certificate against the host name in the URL, so a wrong or poisoned answer
//! cannot impersonate the Matchmaker. Order:
//!
//! 1. the system resolver (normal case, remembered on success);
//! 2. an address remembered from the last success, if it is under
//!    [`FRESH_FOR`] old (a flaky resolver costs no extra delay);
//! 3. DNS over HTTPS to a public resolver reached by IP address, so it needs no
//!    DNS itself (Cloudflare, then Google);
//! 4. any remembered address, however old;
//! 5. the system resolver's error.

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A remembered address is used without asking DoH for this long.
const FRESH_FOR: Duration = Duration::from_secs(600);
const DOH_TIMEOUT: Duration = Duration::from_secs(4);
/// Both are addressed by IP, with a certificate that names the IP.
const DOH_ENDPOINTS: [&str; 2] = ["https://1.1.1.1/dns-query", "https://8.8.8.8/resolve"];
const DNS_TYPE_A: u64 = 1;

type Cache = Mutex<HashMap<String, (Instant, Vec<SocketAddr>)>>;

fn global_cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pass to `ureq::AgentBuilder::resolver`.
pub struct FallbackResolver;

impl ureq::Resolver for FallbackResolver {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        resolve_with(
            netloc,
            global_cache(),
            |name| name.to_socket_addrs().map(|addrs| addrs.collect()),
            doh_lookup,
        )
    }
}

fn resolve_with(
    netloc: &str,
    cache: &Cache,
    system: impl Fn(&str) -> io::Result<Vec<SocketAddr>>,
    doh: impl Fn(&str) -> Vec<IpAddr>,
) -> io::Result<Vec<SocketAddr>> {
    let system_error = match system(netloc) {
        Ok(addrs) if !addrs.is_empty() => {
            remember(cache, netloc, &addrs);
            return Ok(addrs);
        }
        Ok(_) => io::Error::new(io::ErrorKind::NotFound, "the system resolver returned no address"),
        Err(error) => error,
    };

    if let Some(addrs) = recall(cache, netloc, Some(FRESH_FOR)) {
        return Ok(addrs);
    }
    if let Some((host, port)) = split_netloc(netloc) {
        let addrs: Vec<SocketAddr> = doh(&host)
            .into_iter()
            .map(|ip| SocketAddr::new(ip, port))
            .collect();
        if !addrs.is_empty() {
            remember(cache, netloc, &addrs);
            return Ok(addrs);
        }
    }
    if let Some(addrs) = recall(cache, netloc, None) {
        return Ok(addrs);
    }
    Err(system_error)
}

fn remember(cache: &Cache, netloc: &str, addrs: &[SocketAddr]) {
    if let Ok(mut map) = cache.lock() {
        map.insert(netloc.to_string(), (Instant::now(), addrs.to_vec()));
    }
}

fn recall(cache: &Cache, netloc: &str, max_age: Option<Duration>) -> Option<Vec<SocketAddr>> {
    let map = cache.lock().ok()?;
    let (stored_at, addrs) = map.get(netloc)?;
    match max_age {
        Some(limit) if stored_at.elapsed() > limit => None,
        _ => Some(addrs.clone()),
    }
}

/// `host:port` into its parts. None for an IP literal (the system resolver
/// handles those without DNS) or a malformed value.
fn split_netloc(netloc: &str) -> Option<(String, u16)> {
    let (host, port) = netloc.rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    if host.is_empty() || host.starts_with('[') || host.parse::<IpAddr>().is_ok() {
        return None;
    }
    Some((host.to_string(), port))
}

fn doh_lookup(host: &str) -> Vec<IpAddr> {
    for endpoint in DOH_ENDPOINTS {
        let agent = ureq::AgentBuilder::new()
            .timeout(DOH_TIMEOUT)
            .redirects(0)
            .build();
        let response = agent
            .get(endpoint)
            .query("name", host)
            .query("type", "A")
            .set("Accept", "application/dns-json")
            .call();
        if let Ok(response) = response {
            if let Ok(body) = response.into_string() {
                let ips = parse_doh_answer(&body);
                if !ips.is_empty() {
                    return ips;
                }
            }
        }
    }
    Vec::new()
}

/// IPv4 addresses from a DoH JSON answer. CNAME and other records are skipped.
fn parse_doh_answer(body: &str) -> Vec<IpAddr> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(answers) = value.get("Answer").and_then(|answer| answer.as_array()) else {
        return Vec::new();
    };
    answers
        .iter()
        .filter(|record| record.get("type").and_then(|kind| kind.as_u64()) == Some(DNS_TYPE_A))
        .filter_map(|record| record.get("data").and_then(|data| data.as_str()))
        .filter_map(|data| data.parse::<Ipv4Addr>().ok())
        .map(IpAddr::V4)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const NETLOC: &str = "mm.test.example:443";

    fn cache() -> Cache {
        Mutex::new(HashMap::new())
    }

    fn addr(last: u8) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, last)), 443)
    }

    fn dns_failed() -> io::Result<Vec<SocketAddr>> {
        Err(io::Error::other("Hôte inconnu (os error 11001)"))
    }

    #[test]
    fn the_system_resolver_wins_and_is_remembered() {
        let cache = cache();
        let doh_calls = Cell::new(0);
        let got = resolve_with(NETLOC, &cache, |_| Ok(vec![addr(1)]), |_| {
            doh_calls.set(doh_calls.get() + 1);
            vec![]
        })
        .unwrap();
        assert_eq!(got, vec![addr(1)]);
        assert_eq!(doh_calls.get(), 0);
        assert_eq!(recall(&cache, NETLOC, None), Some(vec![addr(1)]));
    }

    #[test]
    fn doh_answers_when_the_system_resolver_fails() {
        let cache = cache();
        let got = resolve_with(NETLOC, &cache, |_| dns_failed(), |host| {
            assert_eq!(host, "mm.test.example");
            vec![IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))]
        })
        .unwrap();
        assert_eq!(got, vec![addr(7)]);
    }

    #[test]
    fn a_fresh_remembered_address_avoids_the_doh_delay() {
        let cache = cache();
        remember(&cache, NETLOC, &[addr(3)]);
        let doh_calls = Cell::new(0);
        let got = resolve_with(NETLOC, &cache, |_| dns_failed(), |_| {
            doh_calls.set(doh_calls.get() + 1);
            vec![]
        })
        .unwrap();
        assert_eq!(got, vec![addr(3)]);
        assert_eq!(doh_calls.get(), 0);
    }

    #[test]
    fn a_stale_remembered_address_is_the_last_resort() {
        let cache = cache();
        cache.lock().unwrap().insert(
            NETLOC.to_string(),
            (Instant::now() - FRESH_FOR - Duration::from_secs(5), vec![addr(9)]),
        );
        let got = resolve_with(NETLOC, &cache, |_| dns_failed(), |_| vec![]).unwrap();
        assert_eq!(got, vec![addr(9)]);
    }

    #[test]
    fn the_system_error_is_returned_when_nothing_works() {
        let cache = cache();
        let error = resolve_with(NETLOC, &cache, |_| dns_failed(), |_| vec![]).unwrap_err();
        assert!(error.to_string().contains("11001"));
    }

    #[test]
    fn an_empty_system_answer_counts_as_a_failure() {
        let cache = cache();
        let got = resolve_with(NETLOC, &cache, |_| Ok(vec![]), |_| {
            vec![IpAddr::V4(Ipv4Addr::new(203, 0, 113, 4))]
        })
        .unwrap();
        assert_eq!(got, vec![addr(4)]);
    }

    #[test]
    fn ip_literals_never_go_to_doh() {
        assert_eq!(split_netloc("203.0.113.5:443"), None);
        assert_eq!(split_netloc("[::1]:443"), None);
        assert_eq!(split_netloc("no-port"), None);
        assert_eq!(
            split_netloc("mm.test.example:8443"),
            Some(("mm.test.example".to_string(), 8443))
        );
    }

    #[test]
    fn doh_json_keeps_only_a_records() {
        let body = r#"{"Status":0,"Answer":[
            {"name":"mm.test.example","type":5,"data":"other.test.example."},
            {"name":"other.test.example","type":1,"data":"203.0.113.10"},
            {"name":"other.test.example","type":1,"data":"not-an-ip"},
            {"name":"other.test.example","type":28,"data":"2001:db8::1"}]}"#;
        assert_eq!(
            parse_doh_answer(body),
            vec![IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))]
        );
        assert!(parse_doh_answer("not json").is_empty());
        assert!(parse_doh_answer(r#"{"Status":3}"#).is_empty());
    }
}
