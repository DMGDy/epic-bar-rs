use std::{
    collections::HashMap,
    fs, io,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    Wired,
    Wireless,
    #[default]
    Offline,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Network {
    pub iface: String,
    pub kind: Kind,
    pub ssid: Option<String>,
    /* Wi-Fi signal in dBm, from `iw dev <iface> link`. */
    pub signal: Option<i32>,
}

/* Signal quality buckets, matching the symbolic icon names. */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    Excellent,
    Good,
    Ok,
    Weak,
    None,
}

impl Strength {
    pub fn from_dbm(dbm: i32) -> Self {
        match dbm {
            -55.. => Self::Excellent,
            -67..=-56 => Self::Good,
            -75..=-68 => Self::Ok,
            -85..=-76 => Self::Weak,
            _ => Self::None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Excellent => "excellent",
            Self::Good => "good",
            Self::Ok => "ok",
            Self::Weak => "weak",
            Self::None => "none",
        }
    }
}

impl Network {
    pub fn label(&self) -> &str {
        match self.kind {
            Kind::Wireless => self.ssid.as_deref().unwrap_or(&self.iface),
            Kind::Wired => &self.iface,
            Kind::Offline => "Offline",
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self.kind {
            Kind::Wired => "network-wired-symbolic",
            Kind::Wireless => match self.signal.map(Strength::from_dbm) {
                Some(Strength::Excellent) => "network-wireless-signal-excellent-symbolic",
                Some(Strength::Good) => "network-wireless-signal-good-symbolic",
                Some(Strength::Ok) => "network-wireless-signal-ok-symbolic",
                Some(Strength::Weak) => "network-wireless-signal-weak-symbolic",
                Some(Strength::None) => "network-wireless-signal-none-symbolic",
                None => "network-wireless-symbolic",
            },
            Kind::Offline => "network-offline-symbolic",
        }
    }

    pub fn tooltip(&self) -> String {
        match self.kind {
            Kind::Wireless => {
                let mut text = match &self.ssid {
                    Some(ssid) => format!("{} - Wi-Fi: {ssid}", self.iface),
                    None => format!("{} - Wi-Fi", self.iface),
                };
                if let Some(dbm) = self.signal {
                    text.push_str(&format!(
                        "\nSignal: {dbm} dBm ({})",
                        Strength::from_dbm(dbm).name()
                    ));
                }
                text
            }
            Kind::Wired => format!("{} - Wired", self.iface),
            Kind::Offline => "Disconnected".to_string(),
        }
    }
}

/* What `iw dev <iface> link` reports for a connected interface. */
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    pub ssid: Option<String>,
    pub signal: Option<i32>,
}

// Signal strength changes quickly; refresh it more often than the old SSID-only cache.
const LINK_TTL: Duration = Duration::from_secs(4);

#[derive(Default)]
pub struct Reader {
    links: HashMap<String, (Instant, Link)>,
}

impl Reader {
    pub fn read(&mut self) -> io::Result<Vec<Network>> {
        let routes = fs::read_to_string("/proc/net/route")?;
        self.read_interfaces(Path::new("/sys/class/net"), &routes)
    }

    fn read_interfaces(&mut self, root: &Path, routes: &str) -> io::Result<Vec<Network>> {
        let mut networks = online_interfaces(root)?;
        if networks.is_empty() {
            self.links.clear();
            return Ok(vec![Network::default()]);
        }
        let primary = primary_iface(routes)?;
        networks.sort_by(|a, b| a.iface.cmp(&b.iface));
        networks.sort_by_key(|net| Some(net.iface.as_str()) != primary.as_deref());
        self.links.retain(|iface, _| {
            networks
                .iter()
                .any(|net| net.iface == *iface && net.kind == Kind::Wireless)
        });
        for network in &mut networks {
            if network.kind == Kind::Wireless {
                let link = self.cached_link(&network.iface);
                network.ssid = link.ssid;
                network.signal = link.signal;
            }
        }
        Ok(networks)
    }

    fn cached_link(&mut self, iface: &str) -> Link {
        if let Some((when, link)) = self.links.get(iface) {
            if when.elapsed() < LINK_TTL {
                return link.clone();
            }
        }
        let link = match fetch_link(iface) {
            Ok(link) => link,
            Err(error) => {
                eprintln!("epic-bar-rs: cannot read Wi-Fi link for {iface}: {error}");
                Link::default()
            }
        };
        self.links
            .insert(iface.to_string(), (Instant::now(), link.clone()));
        link
    }
}

fn link_value(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value.trim().to_string())),
        // A device can disappear between enumeration and reading its link state.
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!("{}: {error}", path.display()),
        )),
    }
}

fn online_interfaces(root: &Path) -> io::Result<Vec<Network>> {
    let mut networks = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.join("device").try_exists()? {
            continue;
        }
        if link_value(&path.join("operstate"))?.as_deref() != Some("up")
            || link_value(&path.join("carrier"))?.as_deref() != Some("1")
        {
            continue;
        }
        let iface = entry.file_name().into_string().map_err(|name| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid interface name: {name:?}"),
            )
        })?;
        let kind = if path.join("wireless").try_exists()? {
            Kind::Wireless
        } else {
            Kind::Wired
        };
        networks.push(Network {
            iface,
            kind,
            ssid: None,
            signal: None,
        });
    }
    Ok(networks)
}

fn primary_iface(routes: &str) -> io::Result<Option<String>> {
    let mut best: Option<(u32, String)> = None;
    for line in routes.lines().skip(1) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid /proc/net/route row",
            ));
        }
        if fields[1] != "00000000" || fields[7] != "00000000" {
            continue;
        }
        let flags = u32::from_str_radix(fields[3], 16).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid route flags: {error}"),
            )
        })?;
        if flags & 1 == 0 {
            continue;
        }
        let metric = fields[6].parse::<u32>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid route metric: {error}"),
            )
        })?;
        if best.as_ref().map_or(true, |(current, _)| metric < *current) {
            best = Some((metric, fields[0].to_string()));
        }
    }
    Ok(best.map(|(_, iface)| iface))
}

fn parse_ssid(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("SSID:")
            .map(str::trim)
            .filter(|ssid| !ssid.is_empty())
            .map(str::to_string)
    })
}

fn parse_signal(text: &str) -> Option<i32> {
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("signal:")?.trim();
        value.split_whitespace().next()?.parse().ok()
    })
}

fn fetch_link(iface: &str) -> io::Result<Link> {
    let output = Command::new("timeout")
        .args(["--kill-after=1s", "2s", "iw", "dev", iface, "link"])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "iw exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(Link {
        ssid: parse_ssid(&text),
        signal: parse_signal(&text),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct Details {
    pub addresses: Vec<String>,
    mac: Option<String>,
    gateways: Vec<String>,
}

impl Details {
    pub fn text(&self) -> String {
        let addresses = if self.addresses.is_empty() {
            "No IPv4/IPv6 addresses assigned".into()
        } else {
            self.addresses.join("\n")
        };
        let mut text = format!("Addresses\n{addresses}");
        if let Some(mac) = &self.mac {
            text.push_str(&format!("\n\nMAC: {mac}"));
        }
        text.push_str("\n\nIPv4 default gateway\n");
        text.push_str(&if self.gateways.is_empty() {
            "No default route".into()
        } else {
            self.gateways.join("\n")
        });
        text
    }
}

#[derive(Deserialize)]
struct AddressInfo {
    local: String,
    prefixlen: u8,
}

#[derive(Deserialize)]
struct LinkInfo {
    ifname: String,
    address: Option<String>,
    addr_info: Vec<AddressInfo>,
}

#[derive(Deserialize)]
struct RouteInfo {
    gateway: Option<String>,
}

pub fn read_details(iface: &str) -> Result<Details, String> {
    let addresses = ip_json(&["-j", "address", "show", "dev", iface])?;
    let routes = ip_json(&["-j", "-4", "route", "show", "default", "dev", iface])?;
    parse_details(iface, &addresses, &routes)
}

fn ip_json(args: &[&str]) -> Result<String, String> {
    let output = Command::new("timeout")
        .args(["--kill-after=1s", "2s", "ip"])
        .args(args)
        .output()
        .map_err(|error| format!("cannot execute ip: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ip exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("invalid ip output: {error}"))
}

fn parse_details(iface: &str, addresses: &str, routes: &str) -> Result<Details, String> {
    let links: Vec<LinkInfo> = serde_json::from_str(addresses)
        .map_err(|error| format!("invalid IP addresses: {error}"))?;
    let link = links
        .into_iter()
        .find(|link| link.ifname == iface)
        .ok_or_else(|| format!("interface {iface} is no longer available"))?;
    let routes: Vec<RouteInfo> =
        serde_json::from_str(routes).map_err(|error| format!("invalid default routes: {error}"))?;
    Ok(Details {
        addresses: link
            .addr_info
            .into_iter()
            .map(|address| format!("{}/{}", address.local, address.prefixlen))
            .collect(),
        mac: link.address,
        gateways: routes
            .into_iter()
            .map(|route| {
                route
                    .gateway
                    .unwrap_or_else(|| "On-link (no gateway)".into())
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn connection_details_include_both_address_families_and_explicit_route_state() {
        let addresses = r#"[{"ifname":"eth0","address":"aa:bb:cc:dd:ee:ff","addr_info":[
            {"local":"192.0.2.3","prefixlen":24},{"local":"2001:db8::3","prefixlen":64}]}]"#;
        let details = parse_details("eth0", addresses, r#"[{"gateway":"192.0.2.1"}]"#).unwrap();
        assert_eq!(details.addresses, ["192.0.2.3/24", "2001:db8::3/64"]);
        assert!(details.text().contains("192.0.2.1"));
        assert!(parse_details("eth0", addresses, "[]")
            .unwrap()
            .text()
            .contains("No default route"));
        assert!(parse_details("eth0", addresses, "[{}]")
            .unwrap()
            .text()
            .contains("On-link"));
        assert!(parse_details("missing", addresses, "[]").is_err());
        assert!(parse_details("eth0", "invalid", "[]").is_err());
    }
    #[test]
    fn route_selection_uses_the_lowest_metric_online_default() {
        let routes = "Iface Destination Gateway Flags RefCnt Use Metric Mask\n\
            eth1 00000000 00000000 0003 0 0 600 00000000\n\
            eth0 00000000 00000000 0003 0 0 100 00000000\n\
            down0 00000000 00000000 0002 0 0 0 00000000\n\
            eth2 00000000 00000000 0001 0 0 0 000000FF\n";
        assert_eq!(primary_iface(routes).unwrap().as_deref(), Some("eth0"));
        assert_eq!(primary_iface("Iface Destination\n").unwrap(), None);
        assert!(primary_iface("Iface Destination\ninvalid\n").is_err());
        assert!(primary_iface("Iface\neth0 00000000 0 0003 0 0 nope 00000000\n").is_err());
    }

    struct Interfaces(std::path::PathBuf);

    impl Interfaces {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "epic-bar-network-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn add(&self, name: &str, physical: bool, state: &str, carrier: &str, wireless: bool) {
            let path = self.0.join(name);
            fs::create_dir(&path).unwrap();
            if physical {
                fs::create_dir(path.join("device")).unwrap();
            }
            if wireless {
                fs::create_dir(path.join("wireless")).unwrap();
            }
            fs::write(path.join("operstate"), state).unwrap();
            fs::write(path.join("carrier"), carrier).unwrap();
        }
    }

    impl Drop for Interfaces {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn enumerates_all_online_physical_links_without_virtual_or_down_devices() {
        let interfaces = Interfaces::new();
        interfaces.add("eth0", true, "up\n", "1\n", false);
        interfaces.add("eth1", true, "up", "1", false);
        interfaces.add("wlan0", true, "up", "1", true);
        interfaces.add("lo", false, "up", "1", false);
        interfaces.add("docker0", false, "up", "1", false);
        interfaces.add("veth0", false, "up", "1", false);
        interfaces.add("unplugged", true, "up", "0", false);
        interfaces.add("radio-off", true, "down", "1", true);
        let mut networks = online_interfaces(&interfaces.0).unwrap();
        networks.sort_by(|a, b| a.iface.cmp(&b.iface));
        assert_eq!(
            networks
                .iter()
                .map(|net| net.iface.as_str())
                .collect::<Vec<_>>(),
            ["eth0", "eth1", "wlan0"]
        );
        assert_eq!(networks[2].kind, Kind::Wireless);
        assert!(online_interfaces(&interfaces.0.join("missing")).is_err());

        let mut reader = Reader::default();
        reader.links.insert(
            "wlan0".into(),
            (
                Instant::now(),
                Link {
                    ssid: Some("Work WiFi".into()),
                    signal: Some(-60),
                },
            ),
        );
        let networks = reader
            .read_interfaces(
                &interfaces.0,
                "Iface Destination Gateway Flags RefCnt Use Metric Mask\n\
            eth0 00000000 0 0003 0 0 600 00000000\n\
            eth1 00000000 0 0003 0 0 100 00000000\n",
            )
            .unwrap();
        assert_eq!(
            networks
                .iter()
                .map(|net| net.iface.as_str())
                .collect::<Vec<_>>(),
            ["eth1", "eth0", "wlan0"]
        );
        assert_eq!(networks[2].label(), "Work WiFi");
        assert_eq!(
            networks[2].icon_name(),
            "network-wireless-signal-good-symbolic"
        );
        let empty = Interfaces::new();
        assert_eq!(
            reader.read_interfaces(&empty.0, "Iface\n").unwrap(),
            [Network::default()]
        );
        assert!(reader.links.is_empty());
    }

    #[test]
    fn ssid_labels_and_caches_are_per_interface() {
        assert_eq!(
            parse_ssid("Connected to 00:00:00:00:00:00\n\tSSID: Work WiFi\n"),
            Some("Work WiFi".to_string())
        );
        assert_eq!(parse_ssid("Not connected.\n"), None);
        assert_eq!(
            parse_signal("Connected to 00:00:00:00:00:00\n\tsignal: -48 dBm\n"),
            Some(-48)
        );
        assert_eq!(parse_signal("Not connected.\n"), None);
        let mut reader = Reader::default();
        for (iface, ssid) in [("wlan0", "First"), ("wlan1", "Second")] {
            reader.links.insert(
                iface.into(),
                (
                    Instant::now(),
                    Link {
                        ssid: Some(ssid.into()),
                        signal: None,
                    },
                ),
            );
        }
        assert_eq!(reader.cached_link("wlan0").ssid.as_deref(), Some("First"));
        assert_eq!(reader.cached_link("wlan1").ssid.as_deref(), Some("Second"));
        assert_eq!(Network::default().label(), "Offline");
        let wireless = Network {
            iface: "wlan0".into(),
            kind: Kind::Wireless,
            ssid: Some("Work WiFi".into()),
            signal: Some(-80),
        };
        assert_eq!(wireless.label(), "Work WiFi");
        assert!(wireless.tooltip().contains("wlan0"));
        assert!(wireless.tooltip().contains("-80 dBm (weak)"));
        assert_eq!(
            wireless.icon_name(),
            "network-wireless-signal-weak-symbolic"
        );
        for (dbm, strength) in [
            (-40, Strength::Excellent),
            (-55, Strength::Excellent),
            (-56, Strength::Good),
            (-70, Strength::Ok),
            (-85, Strength::Weak),
            (-90, Strength::None),
        ] {
            assert_eq!(Strength::from_dbm(dbm), strength, "{dbm} dBm");
        }
    }
}
