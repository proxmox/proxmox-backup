//! Cached traffic control configuration
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::Error;
use cidr::IpInet;

use proxmox_http::client::{ShareableRateLimit, RateLimiter};
use proxmox_section_config::SectionConfigData;

use proxmox_systemd::daily_duration::{parse_daily_duration, DailyDuration};
use proxmox_time::TmEditor;

use pbs_api_types::TrafficControlRule;

use pbs_config::ConfigVersionCache;

use super::SharedRateLimiter;

struct ParsedTcRule {
    config: TrafficControlRule, // original rule config
    networks: Vec<IpInet>, // parsed networks
    timeframe: Vec<DailyDuration>, // parsed timeframe
}

pub struct TrafficControlCache {
    use_shared_memory: bool,
    last_update: i64,
    last_traffic_control_generation: usize,
    rules: Vec<ParsedTcRule>,
    limiter_map: HashMap<String, (Option<Arc<dyn ShareableRateLimit>>, Option<Arc<dyn ShareableRateLimit>>)>,
    use_utc: bool, // currently only used for testing
}

fn timeframe_match(
    duration_list: &[DailyDuration],
    now: &TmEditor,
) -> bool {

    if duration_list.is_empty() { return true; }

    for duration in duration_list.iter() {
        if duration.time_match_with_tm_editor(now) {
            return true;
        }
    }

    false
}

fn network_match_len(
    networks: &[IpInet],
    ip: &IpAddr,
) -> Option<u8> {

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
        IpAddr::V6(addr) => {
            match addr.octets() {
                [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a, b, c, d] => {
                    IpAddr::V4(Ipv4Addr::new(a, b, c, d))
                }
                _ => IpAddr::V6(addr),
            }
        }
    }
}

fn create_limiter(
    use_shared_memory: bool,
    name: &str,
    rate: u64,
    burst: u64,
) -> Result<Arc<dyn ShareableRateLimit>, Error> {
    if use_shared_memory {
        let limiter = SharedRateLimiter::mmap_shmem(name, rate, burst)?;
        Ok(Arc::new(limiter))
    } else {
        Ok(Arc::new(Mutex::new(RateLimiter::new(rate, burst))))
    }
}

impl TrafficControlCache {

    pub fn new() -> Self {
        Self {
            use_shared_memory: true,
            rules: Vec::new(),
            limiter_map: HashMap::new(),
            last_traffic_control_generation: 0,
            last_update: 0,
            use_utc: false,
        }
    }

    pub fn reload(&mut self, now: i64) {
        let version_cache = match ConfigVersionCache::new() {
            Ok(cache) => cache,
            Err(err) => {
                log::error!("TrafficControlCache::reload failed in ConfigVersionCache::new: {}", err);
                return;
            }
        };

        let traffic_control_generation = version_cache.traffic_control_generation();

        if (self.last_update != 0) &&
            (traffic_control_generation == self.last_traffic_control_generation) &&
            ((now - self.last_update) < 60) { return; }

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


    fn update_config(&mut self, config: &SectionConfigData) -> Result<(), Error> {
        self.limiter_map.retain(|key, _value| config.sections.contains_key(key));

        let rules: Vec<TrafficControlRule> =
            config.convert_to_typed_array("rule")?;

        let mut active_rules = Vec::new();

        for rule in rules {

            let entry = self.limiter_map.entry(rule.name.clone()).or_insert((None, None));

            match entry.0 {
                Some(ref read_limiter) => {
                    match rule.rate_in {
                        Some(rate_in) => {
                            read_limiter.update_rate(rate_in, rule.burst_in.unwrap_or(rate_in));
                        }
                        None => entry.0 = None,
                    }
                }
                None => {
                    if let Some(rate_in) = rule.rate_in {
                        let name = format!("{}.in", rule.name);
                        let limiter = create_limiter(
                            self.use_shared_memory,
                            &name,
                            rate_in,
                            rule.burst_in.unwrap_or(rate_in),
                        )?;
                        entry.0 = Some(limiter);
                    }
                }
            }

            match entry.1 {
                Some(ref write_limiter) => {
                    match rule.rate_out {
                        Some(rate_out) => {
                            write_limiter.update_rate(rate_out, rule.burst_out.unwrap_or(rate_out));
                        }
                        None => entry.1 = None,
                    }
                }
                None => {
                    if let Some(rate_out) = rule.rate_out {
                        let name = format!("{}.out", rule.name);
                        let limiter = create_limiter(
                            self.use_shared_memory,
                            &name,
                            rate_out,
                            rule.burst_out.unwrap_or(rate_out),
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

            active_rules.push(ParsedTcRule { config: rule, networks, timeframe });
        }

        self.rules = active_rules;

        Ok(())
    }

    pub fn lookup_rate_limiter(
        &self,
        peer: Option<SocketAddr>,
        now: i64,
    ) -> (&str, Option<Arc<dyn ShareableRateLimit>>, Option<Arc<dyn ShareableRateLimit>>) {

        let peer = match peer {
            None => return ("", None, None),
            Some(peer) => peer,
        };

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
            if !timeframe_match(&rule.timeframe, &now) { continue; }

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
                    Some((read_limiter, write_limiter)) => {
                        (&rule.config.name, read_limiter.clone(), write_limiter.clone())
                    }
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
        (mday*3600*24 + hour*3600 + min*60) as i64
    }

    #[test]
    fn testnetwork_match() -> Result<(), Error> {

        let networks = ["192.168.2.1/24", "127.0.0.0/8"];
        let networks: Vec<IpInet> = networks.iter().map(|n| n.parse().unwrap()).collect();

        assert_eq!(network_match_len(&networks, &"192.168.2.1".parse()?), Some(24));
        assert_eq!(network_match_len(&networks, &"192.168.2.254".parse()?), Some(24));
        assert_eq!(network_match_len(&networks, &"192.168.3.1".parse()?), None);
        assert_eq!(network_match_len(&networks, &"127.1.1.0".parse()?), Some(8));
        assert_eq!(network_match_len(&networks, &"128.1.1.0".parse()?), None);

        let networks = ["0.0.0.0/0"];
        let networks: Vec<IpInet> = networks.iter().map(|n| n.parse().unwrap()).collect();
        assert_eq!(network_match_len(&networks, &"127.1.1.0".parse()?), Some(0));
        assert_eq!(network_match_len(&networks, &"192.168.2.1".parse()?), Some(0));

        Ok(())
    }

    #[test]
    fn test_rule_match()  -> Result<(), Error> {

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

        cache.update_config(&config)?;

        const THURSDAY_80_00: i64 = make_test_time(0, 8, 0);
        const THURSDAY_15_00: i64 = make_test_time(0, 15, 0);
        const THURSDAY_19_00: i64 = make_test_time(0, 19, 0);

        let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 1234);
        let gateway = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 2, 1)), 1234);
        let private = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 2, 35)), 1234);
        let somewhere = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4)), 1234);

        let (rule, read_limiter, write_limiter) = cache.lookup_rate_limiter(None, THURSDAY_80_00);
        assert_eq!(rule, "");
        assert!(read_limiter.is_none());
        assert!(write_limiter.is_none());

        let (rule, read_limiter, write_limiter) = cache.lookup_rate_limiter(Some(somewhere), THURSDAY_80_00);
        assert_eq!(rule, "somewhere");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

         let (rule, read_limiter, write_limiter) = cache.lookup_rate_limiter(Some(local), THURSDAY_19_00);
        assert_eq!(rule, "rule2");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

        let (rule, read_limiter, write_limiter) = cache.lookup_rate_limiter(Some(gateway), THURSDAY_15_00);
        assert_eq!(rule, "rule1");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

        let (rule, read_limiter, write_limiter) = cache.lookup_rate_limiter(Some(gateway), THURSDAY_19_00);
        assert_eq!(rule, "somewhere");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

        let (rule, read_limiter, write_limiter) = cache.lookup_rate_limiter(Some(private), THURSDAY_19_00);
        assert_eq!(rule, "rule2");
        assert!(read_limiter.is_some());
        assert!(write_limiter.is_some());

        Ok(())
    }

}
