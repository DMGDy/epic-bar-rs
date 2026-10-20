use std::{
    collections::HashMap,
    fs, io,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

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
            Kind::Wireless => "network-wireless-symbolic",
            Kind::Offline => "network-offline-symbolic",
        }
    }

    pub fn tooltip(&self) -> String {
        match self.kind {
            Kind::Wireless => match &self.ssid {
                Some(ssid) => format!("{} - Wi-Fi: {ssid}", self.iface),
                None => format!("{} - Wi-Fi", self.iface),
            },
            Kind::Wired => format!("{} - Wired", self.iface),
            Kind::Offline => "Disconnected".to_string(),
        }
    }
}

#[derive(Default)]
pub struct Reader {
    ssids: HashMap<String, (Instant, Option<String>)>,
}

impl Reader {
    pub fn read(&mut self) -> io::Result<Vec<Network>> {
        let routes = fs::read_to_string("/proc/net/route")?;
        self.read_interfaces(Path::new("/sys/class/net"), &routes)
    }

    fn read_interfaces(&mut self, root: &Path, routes: &str) -> io::Result<Vec<Network>> {
        let mut networks = online_interfaces(root)?;
        if networks.is_empty() {
            self.ssids.clear();
            return Ok(vec![Network::default()]);
        }
        let primary = primary_iface(routes)?;
        networks.sort_by(|a, b| a.iface.cmp(&b.iface));
        networks.sort_by_key(|net| Some(net.iface.as_str()) != primary.as_deref());
        self.ssids.retain(|iface, _| {
            networks
                .iter()
                .any(|net| net.iface == *iface && net.kind == Kind::Wireless)
        });
        for network in &mut networks {
            if network.kind == Kind::Wireless {
                network.ssid = self.cached_ssid(&network.iface);
            }
        }
        Ok(networks)
    }

    fn cached_ssid(&mut self, iface: &str) -> Option<String> {
        if let Some((when, ssid)) = self.ssids.get(iface) {
            if when.elapsed() < Duration::from_secs(10) {
                return ssid.clone();
            }
        }
        let ssid = match fetch_ssid(iface) {
            Ok(ssid) => ssid,
            Err(error) => {
                eprintln!("epic-bar-rs: cannot read Wi-Fi SSID for {iface}: {error}");
                None
            }
        };
        self.ssids
            .insert(iface.to_string(), (Instant::now(), ssid.clone()));
        ssid
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

fn fetch_ssid(iface: &str) -> io::Result<Option<String>> {
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
    Ok(parse_ssid(&String::from_utf8_lossy(&output.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
        reader
            .ssids
            .insert("wlan0".into(), (Instant::now(), Some("Work WiFi".into())));
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
        let empty = Interfaces::new();
        assert_eq!(
            reader.read_interfaces(&empty.0, "Iface\n").unwrap(),
            [Network::default()]
        );
        assert!(reader.ssids.is_empty());
    }

    #[test]
    fn ssid_labels_and_caches_are_per_interface() {
        assert_eq!(
            parse_ssid("Connected to 00:00:00:00:00:00\n\tSSID: Work WiFi\n"),
            Some("Work WiFi".to_string())
        );
        assert_eq!(parse_ssid("Not connected.\n"), None);
        let mut reader = Reader::default();
        reader
            .ssids
            .insert("wlan0".into(), (Instant::now(), Some("First".into())));
        reader
            .ssids
            .insert("wlan1".into(), (Instant::now(), Some("Second".into())));
        assert_eq!(reader.cached_ssid("wlan0").as_deref(), Some("First"));
        assert_eq!(reader.cached_ssid("wlan1").as_deref(), Some("Second"));
        assert_eq!(Network::default().label(), "Offline");
        let wireless = Network {
            iface: "wlan0".into(),
            kind: Kind::Wireless,
            ssid: Some("Work WiFi".into()),
        };
        assert_eq!(wireless.label(), "Work WiFi");
        assert!(wireless.tooltip().contains("wlan0"));
    }
}
