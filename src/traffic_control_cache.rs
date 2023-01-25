//! Traffic control implementation

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Error;
use cidr::IpInet;

use proxmox_http::{RateLimiter, ShareableRateLimit};
use proxmox_section_config::SectionConfigData;

use proxmox_time::{parse_daily_duration, DailyDuration, TmEditor};

use pbs_api_types::TrafficControlRule;

use pbs_config::ConfigVersionCache;

use crate::tools::SharedRateLimiter;

pub type SharedRateLimit = Arc<dyn ShareableRateLimit>;

lazy_static::lazy_static! {
    /// Shared traffic control cache singleton.
    pub static ref TRAFFIC_CONTROL_CACHE: Arc<Mutex<TrafficControlCache>> =
        Arc::new(Mutex::new(TrafficControlCache::new()));
}

struct ParsedTcRule {
    config: TrafficControlRule,    // original rule config
    networks: Vec<IpInet>,         // parsed networks
    timeframe: Vec<DailyDuration>, // parsed timeframe
}

/// Traffic control statistics
pub struct TrafficStat {
    /// Total incoming traffic (bytes)
    pub traffic_in: u64,
    /// Incoming data rate (bytes/second)
    pub rate_in: u64,
    /// Total outgoing traffic (bytes)
    pub traffic_out: u64,
    /// Outgoing data rate (bytes/second)
    pub rate_out: u64,
}

/// Cache rules from `/etc/proxmox-backup/traffic-control.cfg`
/// together with corresponding rate limiter implementation.
pub struct TrafficControlCache {
    // use shared memory to make it work with daemon restarts
    use_shared_memory: bool,
    last_rate_compute: Instant,
    current_rate_map: HashMap<String, TrafficStat>,
    last_update: i64,
    last_traffic_control_generation: usize,
    rules: Vec<ParsedTcRule>,
    limiter_map: HashMap<String, (Option<SharedRateLimit>, Option<SharedRateLimit>)>,
    use_utc: bool, // currently only used for testing
}

fn timeframe_match(duration_list: &[DailyDuration], now: &TmEditor) -> bool {
    if duration_list.is_empty() {
        return true;
    }

    for duration in duration_list.iter() {
        if duration.time_match_with_tm_editor(now) {
            return true;
        }
    }

    false
}

fn network_match_len(networks: &[IpInet], ip: &IpAddr) -> Option<u8> {
    let mut match_len = None;

    for cidr in networks.iter() {
        if cidr.contains(ip) {
            let network_length = cidr.network_length();
            match match_len {
                Some(len) => {
                    if network_length > len {
                        match_len = Some(network_length);
                    }
                }
                None => match_len = Some(network_length),
            }
        }
    }
    match_len
}

fn cannonical_ip(ip: IpAddr) -> IpAddr {
    // TODO: use std::net::IpAddr::to_cananical once stable
    match ip {
        IpAddr::V4(addr) => IpAddr::V4(addr),
        IpAddr::V6(addr) => match addr.octets() {
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a, b, c, d] => {
                IpAddr::V4(Ipv4Addr::new(a, b, c, d))
            }
            _ => IpAddr::V6(addr),
        },
    }
}

fn create_limiter(
    use_shared_memory: bool,
    name: &str,
    rate: u64,
    burst: u64,
) -> Result<SharedRateLimit, Error> {
    if use_shared_memory {
        let limiter = SharedRateLimiter::mmap_shmem(name, rate, burst)?;
        Ok(Arc::new(limiter))
    } else {
        Ok(Arc::new(Mutex::new(RateLimiter::new(rate, burst))))
    }
}

impl TrafficControlCache {
    fn new() -> Self {
        Self {
            use_shared_memory: true,
            rules: Vec::new(),
            limiter_map: HashMap::new(),
            last_traffic_control_generation: 0,
            last_update: 0,
            use_utc: false,
            last_rate_compute: Instant::now(),
            current_rate_map: HashMap::new(),
        }
    }

    /// Reload rules from configuration file
    ///
    /// Only reload if configuration file was updated
    /// ([ConfigVersionCache]) or last update is older that 60
    /// seconds.
    pub fn reload(&mut self, now: i64) {
        let version_cache = match ConfigVersionCache::new() {
            Ok(cache) => cache,
            Err(err) => {
                log::error!(
                    "TrafficControlCache::reload failed in ConfigVersionCache::new: {}",
                    err
                );
                return;
            }
        };

        let traffic_control_generation = version_cache.traffic_control_generation();

        if (self.last_update != 0)
            && (traffic_control_generation == self.last_traffic_control_generation)
            && ((now - self.last_update) < 60)
        {
            return;
        }

        log::debug!("reload traffic control rules");

        self.last_traffic_control_generation = traffic_control_generation;
        self.last_update = now;

        match self.reload_impl() {
            Ok(()) => (),
            Err(err) => {
                log::error!("TrafficControlCache::reload failed -> {}", err);
            }
        }
    }

    fn reload_impl(&mut self) -> Result<(), Error> {
        let (config, _) = pbs_config::traffic_control::config()?;

        self.update_config(&config)
    }

    /// Compute current data rates.
    ///
    /// This should be called every second (from `proxmox-backup-proxy`).
    pub fn compute_current_rates(&mut self) {
        let elapsed = self.last_rate_compute.elapsed().as_micros();
        if elapsed < 200_000 {
            return;
        } // not enough data

        let mut new_rate_map = HashMap::new();

        for (rule, (read_limit, write_limit)) in self.limiter_map.iter() {
            let traffic_in = read_limit.as_ref().map(|l| l.traffic()).unwrap_or(0);
            let traffic_out = write_limit.as_ref().map(|l| l.traffic()).unwrap_or(0);

            let traffic_diff_in;
            let traffic_diff_out;

            if let Some(stat) = self.current_rate_map.get(rule) {
                traffic_diff_in = traffic_in.saturating_sub(stat.traffic_in);
                traffic_diff_out = traffic_out.saturating_sub(stat.traffic_out);
            } else {
                traffic_diff_in = 0;
                traffic_diff_out = 0;
            }

            let rate_in = ((traffic_diff_in as u128) * 1_000_000) / elapsed;
            let rate_out = ((traffic_diff_out as u128) * 1_000_000) / elapsed;

            let stat = TrafficStat {
                traffic_in,
                traffic_out,
                rate_in: rate_in.try_into().unwrap_or(u64::MAX),
                rate_out: rate_out.try_into().unwrap_or(u64::MAX),
            };
            new_rate_map.insert(rule.clone(), stat);
        }

        self.current_rate_map = new_rate_map;

        self.last_rate_compute = Instant::now()
    }

    /// Returns current [TrafficStat] for each configured rule.
    pub fn current_rate_map(&self) -> &HashMap<String, TrafficStat> {
        &self.current_rate_map
    }

    fn update_config(&mut self, config: &SectionConfigData) -> Result<(), Error> {
        self.limiter_map
            .retain(|key, _value| config.sections.contains_key(key));

        let rules: Vec<TrafficControlRule> = config.convert_to_typed_array("rule")?;

        let mut active_rules = Vec::new();

        for rule in rules {
            let entry = self
                .limiter_map
                .entry(rule.name.clone())
                .or_insert((None, None));
            let limit = &rule.limit;

            match entry.0 {
                Some(ref read_limiter) => match limit.rate_in {
                    Some(rate_in) => {
                        read_limiter.update_rate(
                            rate_in.as_u64(),
                            limit.burst_in.unwrap_or(rate_in).as_u64(),
                        );
                    }
                    None => entry.0 = None,
                },
                None => {
                    if let Some(rate_in) = limit.rate_in {
                        let name = format!("{}.in", rule.name);
                        let limiter = create_limiter(
                            self.use_shared_memory,
                            &name,
                            rate_in.as_u64(),
                            limit.burst_in.unwrap_or(rate_in).as_u64(),
                        )?;
                        entry.0 = Some(limiter);
                    }
                }
            }

            match entry.1 {
                Some(ref write_limiter) => match limit.rate_out {
                    Some(rate_out) => {
                        write_limiter.update_rate(
                            rate_out.as_u64(),
                            limit.burst_out.unwrap_or(rate_out).as_u64(),
                        );
                    }
                    None => entry.1 = None,
                },
                None => {
                    if let Some(rate_out) = limit.rate_out {
                        let name = format!("{}.out", rule.name);
                        let limiter = create_limiter(
                            self.use_shared_memory,
                            &name,
                            rate_out.as_u64(),
                            limit.burst_out.unwrap_or(rate_out).as_u64(),
                        )?;
                        entry.1 = Some(limiter);
                    }
                }
            }

            let mut timeframe = Vec::new();

            if let Some(ref timefram_list) = rule.timeframe {
                for duration_str in timefram_list {
                    let duration = parse_daily_duration(duration_str)?;
                    timeframe.push(duration);
                }
            }

            let mut networks = Vec::new();

            for network in rule.network.iter() {
                let cidr = match network.parse() {
                    Ok(cidr) => cidr,
                    Err(err) => {
                        log::error!("unable to parse network '{}' - {}", network, err);
                        continue;
                    }
                };
                networks.push(cidr);
            }

            active_rules.push(ParsedTcRule {
                config: rule,
                networks,
                timeframe,
            });
        }

        self.rules = active_rules;

        Ok(())
    }

    /// Returns the rate limiter (if any) for the specified peer address.
    ///
    /// - Rules where timeframe does not match are skipped.
    /// - Rules with smaller network size have higher priority.
    ///
    /// Behavior is undefined if more than one rule matches after
    /// above selection.
    pub fn lookup_rate_limiter(
        &self,
        peer: SocketAddr,
        now: i64,
    ) -> (&str, Option<SharedRateLimit>, Option<SharedRateLimit>) {
        let peer_ip = cannonical_ip(peer.ip());

        log::debug!("lookup_rate_limiter: {:?}", peer_ip);

        let now = match TmEditor::with_epoch(now, self.use_utc) {
            Ok(now) => now,
            Err(err) => {
                log::error!("lookup_rate_limiter: TmEditor::with_epoch failed - {}", err);
                return ("", None, None);
            }
        };

        let mut last_rule_match = None;

        for rule in self.rules.iter() {
            if !timeframe_match(&rule.timeframe, &now) {
                continue;
            }

            if let Some(match_len) = network_match_len(&rule.networks, &peer_ip) {
                match last_rule_match {
                    None => last_rule_match = Some((rule, match_len)),
                    Some((_, last_len)) => {
                        if match_len > last_len {
                            last_rule_match = Some((rule, match_len));
                        }
                    }
                }
            }
        }

        match last_rule_match {
            Some((rule, _)) => {
                match self.limiter_map.get(&rule.config.name) {
                    Some((read_limiter, write_limiter)) => (
                        &rule.config.name,
                        read_limiter.clone(),
                        write_limiter.clone(),
                    ),
                    None => ("", None, None), // should never happen
                }
            }
            None => ("", None, None),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const fn make_test_time(mday: i32, hour: i32, min: i32) -> i64 {
        (mday * 3600 * 24 + hour * 3600 + min * 60) as i64
    }

    #[test]
    fn testnetwork_match() -> Result<(), Error> {
        let networks = ["192.168.2.1/24", "127.0.0.0/8"];
        let networks: Vec<IpInet> = networks.iter().map(|n| n.parse().unwrap()).collect();

        assert_eq!(
            network_match_len(&networks, &"192.168.2.1".parse()?),
            Some(24)
        );
        assert_eq!(
            network_match_len(&networks, &"192.168.2.254".parse()?),
            Some(24)
        );
        assert_eq!(network_match_len(&networks, &"192.168.3.1".parse()?), None);
        assert_eq!(network_match_len(&networks, &"127.1.1.0".parse()?), Some(8));
        assert_eq!(network_match_len(&networks, &"128.1.1.0".parse()?), None);

        let networks = ["0.0.0.0/0"];
        let networks: Vec<IpInet> = networks.iter().map(|n| n.parse().unwrap()).collect();
        assert_eq!(network_match_len(&networks, &"127.1.1.0".parse()?), Some(0));
        assert_eq!(
            network_match_len(&networks, &"192.168.2.1".parse()?),
            Some(0)
        );

        Ok(())
    }

    #[test]
    fn test_rule_match() -> Result<(), Error> {
        let config_data = "
rule: rule1
	comment my test rule
	network 192.168.2.0/24
	rate-in 50000000
	rate-out 50000000
	timeframe 8-12
	timeframe 14-16

rule: rule2
	network 192.168.2.35/32
	network 127.0.0.1/8
	rate-in 150000000
	rate-out 150000000
	timeframe 18-20

rule: somewhere
	network 0.0.0.0/0
	rate-in 100000000
	rate-out 100000000
";
        let config = pbs_config::traffic_control::CONFIG.parse("testconfig", config_data)?;

        let mut cache = TrafficControlCache::new();
        cache.use_utc = true;
        cache.use_shared_memory = false; // avoid permission problems in test environment

        cache.update_config(&config)?;

        const THURSDAY_80_00: i64 = make_test_time(0, 8, 0);
        const THURSDAY_15_00: i64 = make_test_time(0, 15, 0);
        const THURSDAY_19_00: i64 = make_test_time(0, 19, 0);

        let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1234);
        let gateway = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 2, 1)), 1234);
        let private = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 2, 35)), 1234);
        let somewhere = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), 1234);

        let (rule, read_limiter, write_limiter) =
            cache.lookup_rate_limiter(somewhere, THURSDAY_80_00);
        assert_eq!(rule, "somewhere");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

        let (rule, read_limiter, write_limiter) = cache.lookup_rate_limiter(local, THURSDAY_19_00);
        assert_eq!(rule, "rule2");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

        let (rule, read_limiter, write_limiter) =
            cache.lookup_rate_limiter(gateway, THURSDAY_15_00);
        assert_eq!(rule, "rule1");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

        let (rule, read_limiter, write_limiter) =
            cache.lookup_rate_limiter(gateway, THURSDAY_19_00);
        assert_eq!(rule, "somewhere");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

        let (rule, read_limiter, write_limiter) =
            cache.lookup_rate_limiter(private, THURSDAY_19_00);
        assert_eq!(rule, "rule2");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

        Ok(())
    }
}
