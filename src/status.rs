use std::{
    fmt, fs,
    path::{Path, PathBuf},
    thread,
};

use time::{format_description, OffsetDateTime};

const BATTERY_PERCENTAGE: &str = "/sys/class/power_supply/BAT1/capacity";
const BATTERY_STATUS: &str = "/sys/class/power_supply/BAT1/status";
const BATTERY_CHARGE: &str = "/sys/class/power_supply/BAT1/charge_now";
const BATTERY_CHARGE_FULL: &str = "/sys/class/power_supply/BAT1/charge_full";
const BATTERY_CURRENT: &str = "/sys/class/power_supply/BAT1/current_now";

const MEMORY_INFO: &str = "/proc/meminfo";

const CPU_STAT: &str = "/proc/stat";
// ex. path: /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq
const CPU_FREQ_PREF: &str = "/sys/devices/system/cpu/cpu"; //followed by cpu number
const CPU_FREQ_POST: &str = "/cpufreq/scaling_cur_freq";

#[derive(Clone)]
pub struct DateTime {
    pub date: String,
    pub time: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusLevel {
    Normal,
    Warning,
    Critical,
}

impl StatusLevel {
    fn usage(percent: f64, warning: f64, critical: f64) -> Self {
        if percent >= critical {
            Self::Critical
        } else if percent >= warning {
            Self::Warning
        } else {
            Self::Normal
        }
    }

    pub fn cpu(percent: f32) -> Self {
        Self::usage(f64::from(percent), 85.0, 95.0)
    }

    pub fn memory(percent: f64) -> Self {
        Self::usage(percent, 80.0, 90.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum BatteryStatus {
    Discharging,
    Charging,
    NotCharging,
    Error,
}

#[derive(Clone)]
pub struct Battery {
    pub capacity: u32, //percentage left on battery
    pub icon: String,
    pub tooltip_text: String, // formatted with remaining
    remaining: String,

    status: BatteryStatus,
}

impl Default for Battery {
    fn default() -> Self {
        Battery {
            capacity: 0,
            icon: String::new(),
            remaining: String::new(),
            tooltip_text: String::new(),
            status: BatteryStatus::Error,
        }
    }
}

impl Battery {
    pub fn is_charging(&self) -> bool {
        self.status == BatteryStatus::Charging
    }

    pub fn level(&self) -> StatusLevel {
        match self.status {
            BatteryStatus::Error => StatusLevel::Warning,
            BatteryStatus::Discharging if self.capacity <= 10 => StatusLevel::Critical,
            BatteryStatus::Discharging if self.capacity <= 20 => StatusLevel::Warning,
            _ => StatusLevel::Normal,
        }
    }
}

#[derive(Clone)]
pub struct Memory {
    pub string: String,
    pub used_percent: Option<f64>,
}

#[derive(Clone, Copy)]
pub struct Cpu {
    total_load1: u64,
    non_idle_load1: u64,
    total_load2: u64,
    non_idle_load2: u64,

    pub avg_load: f32,
}

fn read_trimmed(path: &str) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Some(contents.trim_end().to_string()),
        Err(error) => {
            eprintln!("epic-bar-rs: cannot read {path}: {error}");
            None
        }
    }
}

/* Enumerate the scaling_cur_freq path of every online CPU core.
 * The core count is detected at runtime instead of hardcoded, so machines
 * with any number of cores work and a missing core never causes a panic.
 */
fn cpu_freq_paths() -> Vec<String> {
    let mut paths = Vec::new();
    let mut n = 0;
    loop {
        let path = format!("{CPU_FREQ_PREF}{n}{CPU_FREQ_POST}");
        if Path::new(&path).exists() {
            paths.push(path);
            n += 1;
        } else {
            break;
        }
    }
    paths
}

// read the aggregate cpu line of /proc/stat -> (total_time, non_idle_time)
fn read_cpu_totals() -> Option<(u64, u64)> {
    let contents = read_trimmed(CPU_STAT)?;
    match parse_cpu_totals(&contents) {
        Ok(totals) => Some(totals),
        Err(error) => {
            eprintln!("epic-bar-rs: invalid CPU statistics: {error}");
            None
        }
    }
}

fn parse_cpu_totals(contents: &str) -> Result<(u64, u64), String> {
    let mut fields = contents
        .lines()
        .next()
        .ok_or("missing CPU totals")?
        .split_whitespace();
    if fields.next() != Some("cpu") {
        return Err("missing aggregate CPU row".to_string());
    }
    let values = fields
        .take(8)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    if values.len() != 8 {
        return Err("incomplete CPU totals".to_string());
    }
    // Exclude guest counters, which are already included in user/nice.
    let total = values
        .iter()
        .try_fold(0u64, |sum, value| sum.checked_add(*value))
        .ok_or("CPU totals overflow")?;
    let idle = values[3] + values[4];
    Ok((total, total - idle))
}

impl Cpu {
    pub fn new() -> Self {
        // fall back to zeros if /proc/stat can't be read; avg starts at 0 anyway
        let (total_load, non_idle_load) = read_cpu_totals().unwrap_or((0, 0));

        Cpu {
            total_load1: total_load,
            non_idle_load1: non_idle_load,
            total_load2: 0,
            non_idle_load2: 0,
            avg_load: 0.0,
        }
    }

    pub fn get_avg_freq() -> String {
        let mut handles = Vec::new();

        // each handle reads one core's current frequency in Hz (None on failure)
        for path in cpu_freq_paths() {
            let handle = thread::spawn(move || -> Option<f32> {
                let value = read_trimmed(&path)?;
                match value.parse::<f32>() {
                    Ok(frequency) if frequency.is_finite() && frequency > 0.0 => Some(frequency),
                    _ => {
                        eprintln!("epic-bar-rs: invalid CPU frequency in {path}: {value}");
                        None
                    }
                }
            });
            handles.push(handle);
        }

        // average only over cores read successfully; divisor is the live count
        let freqs: Vec<f32> = handles
            .into_iter()
            .filter_map(|h| match h.join() {
                Ok(frequency) => frequency,
                Err(error) => {
                    eprintln!("epic-bar-rs: CPU frequency reader panicked: {error:?}");
                    None
                }
            })
            .collect();

        if freqs.is_empty() {
            return "N/A".to_string();
        }

        let total: f32 = freqs.iter().sum();
        let freq_avg = total / (freqs.len() as f32 * 1_000_000.0);
        if freq_avg < 1.0 {
            format!("{:.0} MHz", freq_avg * 1000.0)
        } else {
            format!("{:.1} GHz", freq_avg)
        }
    }

    pub fn get_cpu_load(&mut self) -> f32 {
        // keep the last known load if /proc/stat can't be read this tick
        let (total_load, non_idle_load) = match read_cpu_totals() {
            Some(v) => v,
            None => return self.avg_load,
        };

        // delta between total time AND non-idle times
        // ((total_time - non_idle_time) / total_time) * 100 = avg load
        self.total_load2 = self.total_load1;
        self.non_idle_load2 = self.non_idle_load1;

        self.total_load1 = total_load;
        self.non_idle_load1 = non_idle_load;

        let delta_total = self.total_load1.saturating_sub(self.total_load2);
        let delta_non_idle = self.non_idle_load1.saturating_sub(self.non_idle_load2);

        // identical samples would divide by zero; keep the last value instead
        if delta_total == 0 {
            return self.avg_load;
        }

        self.avg_load = (delta_non_idle as f32 / delta_total as f32) * 100.0;
        self.avg_load
    }
}

fn get_battery(b: &mut Battery) {
    if let Some(cap) = read_trimmed(BATTERY_PERCENTAGE).and_then(|s| s.parse::<u32>().ok()) {
        b.capacity = cap;
    }
}

fn get_status(b: &mut Battery) {
    b.status = match read_trimmed(BATTERY_STATUS).as_deref() {
        Some("Charging") => BatteryStatus::Charging,
        Some("Discharging") => BatteryStatus::Discharging,
        Some("Not charging") => BatteryStatus::NotCharging,
        _ => BatteryStatus::Error,
    };
}

fn get_battery_tooltip_text(b: &mut Battery) {
    let status_text = match b.status {
        BatteryStatus::Charging => "Charging",
        BatteryStatus::Discharging => "Discharging",
        BatteryStatus::NotCharging => "Plugged in, Not Charging",
        _ => "Error getting state",
    };

    b.tooltip_text = format!("{status_text}\n{}", b.remaining);
}

// TODO: not considering charging until full time
fn get_remaining(b: &mut Battery) {
    // read current charge
    match &b.status {
        state @ (BatteryStatus::Charging | BatteryStatus::Discharging) => {
            let charge_now = match read_trimmed(BATTERY_CHARGE).and_then(|s| s.parse::<f64>().ok())
            {
                Some(c) => c,
                None => {
                    b.remaining = "Unknown time remaining".to_string();
                    return;
                }
            };

            // get current_now
            let current = read_trimmed(BATTERY_CURRENT)
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(0.0f64);

            // a zero draw would divide by zero; report indeterminate instead
            if current == 0.0 {
                b.remaining = "Calculating time remaining".to_string();
                return;
            }

            match state {
                BatteryStatus::Charging => {
                    // get full charge of battery to find remaining charge
                    let charge_full = match read_trimmed(BATTERY_CHARGE_FULL)
                        .and_then(|s| s.parse::<f64>().ok())
                    {
                        Some(f) => f,
                        None => {
                            b.remaining = "Unknown time to full".to_string();
                            return;
                        }
                    };
                    let charge_remaining = charge_full - charge_now;
                    let t = (charge_remaining * 3600f64) / current;
                    let t_h = t / 3600f64;
                    let t_m = (t % 3600f64) / 60f64;

                    if t_h >= 1.0 {
                        b.remaining =
                            format!("{} hours {} minutes to full", t_h as u32, t_m as u32)
                                .to_string();
                    } else {
                        b.remaining = format!("{} minutes to full", t_m as u32).to_string();
                    }
                }
                BatteryStatus::Discharging => {
                    // time remaining in seconds
                    let t: f64 = (charge_now * 3600f64) / current;
                    let t_h: f64 = t / 3600f64;
                    let t_m: f64 = (t % 3600f64) / 60f64;

                    if t_h >= 1.0 {
                        b.remaining =
                            format!("{} hours {} minutes remaining", t_h as u32, t_m as u32)
                                .to_string();
                    } else {
                        b.remaining = format!("{} minutes remaining", t_m as u32).to_string();
                    }
                }
                _ => (),
            };
        }
        BatteryStatus::NotCharging => b.remaining = "Fully charged".to_string(),
        _ => b.remaining = "Error getting state".to_string(),
    };
}

fn get_icon(b: &mut Battery) {
    let level = match b.capacity {
        0..=10 => "caution",
        11..=30 => "low",
        31..=80 => "good",
        _ => "full",
    };
    b.icon = match b.status {
        BatteryStatus::Error => "battery-missing-symbolic".to_string(),
        BatteryStatus::Charging => format!("battery-{level}-charging-symbolic"),
        _ => format!("battery-{level}-symbolic"),
    };
}

pub fn get_battery_info() -> Battery {
    let mut battery = Battery::default();
    // in this order
    get_battery(&mut battery);
    get_status(&mut battery);
    get_icon(&mut battery);
    get_remaining(&mut battery);

    get_battery_tooltip_text(&mut battery);
    battery
}

/* One status-timer tick, shared by every output's bar. */
#[derive(Clone)]
pub struct Sample {
    pub battery: Option<Battery>,
    pub datetime: DateTime,
    pub memory: Memory,
    pub cpu_load: f32,
    /* Only sampled while some bar shows CPU details. */
    pub cpu_freq: Option<String>,
    pub cpu_temperature: Option<f64>,
}

/* The configured clock rows. Formats are validated when the config loads. */
pub struct Clock {
    date: format_description::OwnedFormatItem,
    time: format_description::OwnedFormatItem,
}

impl Clock {
    pub fn new(config: &crate::config::Clock) -> Result<Self, String> {
        let parse = |format: &str| {
            format_description::parse_owned::<2>(format)
                .map_err(|error| format!("invalid clock format {format:?}: {error}"))
        };
        Ok(Self {
            date: parse(&config.date)?,
            time: parse(&config.time)?,
        })
    }

    pub fn format(&self, dt: &OffsetDateTime) -> DateTime {
        let format = |item| {
            dt.format(item).unwrap_or_else(|error| {
                eprintln!("epic-bar-rs: cannot format clock: {error}");
                "--".into()
            })
        };
        DateTime {
            date: format(&self.date),
            time: format(&self.time),
        }
    }

    pub fn now(&self) -> DateTime {
        // fall back to UTC if the local offset can't be determined
        self.format(&OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc()))
    }
}

/* CPU package temperature from hwmon: k10temp (AMD), coretemp (Intel),
 * zenpower, or an ARM cpu_thermal zone. The sensor path is found once. */
#[derive(Default)]
pub struct Thermal {
    path: Option<Option<PathBuf>>,
}

const CPU_SENSORS: [(&str, &[&str]); 4] = [
    ("k10temp", &["Tctl", "Tdie"]),
    ("zenpower", &["Tdie", "Tctl"]),
    ("coretemp", &["Package id 0"]),
    ("cpu_thermal", &[]),
];

fn find_cpu_sensor(root: &Path) -> Option<PathBuf> {
    let mut devices: Vec<_> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    devices.sort();
    for (driver, labels) in CPU_SENSORS {
        for device in &devices {
            let name = fs::read_to_string(device.join("name")).unwrap_or_default();
            if name.trim() != driver {
                continue;
            }
            for label in labels.iter() {
                for index in 1..=16 {
                    let path = device.join(format!("temp{index}_label"));
                    if fs::read_to_string(&path).is_ok_and(|text| text.trim() == *label) {
                        return Some(device.join(format!("temp{index}_input")));
                    }
                }
            }
            let input = device.join("temp1_input");
            if input.exists() {
                return Some(input);
            }
        }
    }
    None
}

pub fn read_millidegrees(path: &Path) -> Option<f64> {
    let value = fs::read_to_string(path).ok()?.trim().parse::<f64>().ok()?;
    Some(value / 1000.0).filter(|celsius| (-50.0..200.0).contains(celsius))
}

impl Thermal {
    pub fn celsius(&mut self) -> Option<f64> {
        let path = self
            .path
            .get_or_insert_with(|| find_cpu_sensor(Path::new("/sys/class/hwmon")));
        read_millidegrees(path.as_deref()?)
    }
}

// so can be easily format out
impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n{}", self.date, self.time)
    }
}

pub fn get_mem_info() -> Memory {
    read_mem_info().unwrap_or_else(|| Memory {
        string: "N/A".to_string(),
        used_percent: None,
    })
}

fn read_mem_info() -> Option<Memory> {
    let contents = read_trimmed(MEMORY_INFO)?;
    match parse_memory(&contents) {
        Ok(memory) => Some(memory),
        Err(error) => {
            eprintln!("epic-bar-rs: invalid memory statistics: {error}");
            None
        }
    }
}

fn parse_memory(contents: &str) -> Result<Memory, String> {
    let field = |name: &str| -> Result<f64, String> {
        let value = contents
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().next())
            .ok_or_else(|| format!("missing {name}"))?;
        value
            .parse::<f64>()
            .map_err(|error| format!("{name}: {error}"))
    };
    let memtotal_kb = field("MemTotal:")?;
    let available_kb = field("MemAvailable:")?;
    if !memtotal_kb.is_finite()
        || memtotal_kb <= 0.0
        || !available_kb.is_finite()
        || !(0.0..=memtotal_kb).contains(&available_kb)
    {
        return Err("invalid total/available memory".to_string());
    }
    let total = memtotal_kb / 1_048_576.0;
    let used = (memtotal_kb - available_kb) / 1_048_576.0;

    let string = format!("{:.1}/{:.1} GiB", used, total).to_string();

    Ok(Memory {
        string,
        used_percent: Some((memtotal_kb - available_kb) / memtotal_kb * 100.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_clock_formats_both_rows_from_the_full_timestamp() {
        let dt = time::Date::from_calendar_date(2026, time::Month::October, 2)
            .unwrap()
            .with_hms(17, 6, 9)
            .unwrap()
            .assume_utc();
        let clock = Clock::new(&crate::config::Clock {
            date: "[weekday repr:short] [day] [month repr:short]".into(),
            time: "[hour repr:12]:[minute] [period]".into(),
        })
        .unwrap();
        let formatted = clock.format(&dt);
        assert_eq!(formatted.date, "Fri 02 Oct");
        assert_eq!(formatted.time, "05:06 PM");
        let default = Clock::new(&crate::config::Clock::default()).unwrap();
        assert_eq!(default.format(&dt).to_string(), "2026/10/02\n17:06:09");
    }

    #[test]
    fn cpu_sensor_prefers_labelled_package_temperature() {
        let root = std::env::temp_dir().join(format!("epic-bar-hwmon-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (dir, name) in [("hwmon0", "nvme"), ("hwmon1", "k10temp")] {
            fs::create_dir_all(root.join(dir)).unwrap();
            fs::write(root.join(dir).join("name"), format!("{name}\n")).unwrap();
        }
        fs::write(root.join("hwmon0/temp1_input"), "40000").unwrap();
        fs::write(root.join("hwmon1/temp1_label"), "Tccd1").unwrap();
        fs::write(root.join("hwmon1/temp1_input"), "61000").unwrap();
        fs::write(root.join("hwmon1/temp3_label"), "Tctl\n").unwrap();
        fs::write(root.join("hwmon1/temp3_input"), "52500\n").unwrap();
        let sensor = find_cpu_sensor(&root).unwrap();
        assert_eq!(sensor, root.join("hwmon1/temp3_input"));
        assert_eq!(read_millidegrees(&sensor), Some(52.5));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clock_format_keeps_zero_padded_date_and_time() {
        let dt = time::Date::from_calendar_date(2026, time::Month::October, 2)
            .unwrap()
            .with_hms(7, 6, 9)
            .unwrap()
            .assume_utc();
        let clock = Clock::new(&crate::config::Clock::default())
            .unwrap()
            .format(&dt);
        assert_eq!(clock.date, "2026/10/02");
        assert_eq!(clock.time, "07:06:09");
    }

    #[test]
    fn cpu_counters_are_not_truncated_or_limited_to_u32() {
        let sample = "cpu 5000000000 2000000000 3000000000 4000000000 500000000 60 70 80 0 0\n";
        assert_eq!(
            parse_cpu_totals(sample).unwrap(),
            (14_500_000_210, 10_000_000_210)
        );
        assert!(parse_cpu_totals("cpu 1 nope 3 4 5 6 7 8\n").is_err());
        assert!(parse_cpu_totals("cpu 1 2\n").is_err());
    }

    #[test]
    fn memory_stats_keep_percentage_and_available_memory_semantics() {
        let memory =
            parse_memory("MemAvailable: 2097152 kB\nMemFree: 0 kB\nMemTotal: 8388608 kB\n")
                .unwrap();
        assert_eq!(memory.string, "6.0/8.0 GiB");
        assert_eq!(memory.used_percent, Some(75.0));
        assert!(parse_memory("MemTotal: 0 kB\nMemAvailable: 0 kB\n").is_err());
        assert!(parse_memory("MemTotal: 1024 kB\n").is_err());
    }

    #[test]
    fn usage_thresholds_are_inclusive() {
        for (percent, expected) in [
            (0.0, StatusLevel::Normal),
            (84.9, StatusLevel::Normal),
            (85.0, StatusLevel::Warning),
            (94.9, StatusLevel::Warning),
            (95.0, StatusLevel::Critical),
            (100.0, StatusLevel::Critical),
        ] {
            assert_eq!(StatusLevel::cpu(percent), expected);
        }
        for (percent, expected) in [
            (79.9, StatusLevel::Normal),
            (80.0, StatusLevel::Warning),
            (89.9, StatusLevel::Warning),
            (90.0, StatusLevel::Critical),
        ] {
            assert_eq!(StatusLevel::memory(percent), expected);
        }
    }

    #[test]
    fn battery_alerts_only_when_discharging_or_unknown() {
        let mut battery = Battery::default();
        assert_eq!(battery.level(), StatusLevel::Warning);
        assert!(!battery.is_charging());
        battery.status = BatteryStatus::Discharging;
        assert!(!battery.is_charging());
        for (capacity, expected) in [
            (0, StatusLevel::Critical),
            (10, StatusLevel::Critical),
            (11, StatusLevel::Warning),
            (20, StatusLevel::Warning),
            (21, StatusLevel::Normal),
        ] {
            battery.capacity = capacity;
            assert_eq!(battery.level(), expected);
        }
        battery.capacity = 5;
        battery.status = BatteryStatus::Charging;
        assert_eq!(battery.level(), StatusLevel::Normal);
        assert!(battery.is_charging());
        get_icon(&mut battery);
        assert_eq!(battery.icon, "battery-caution-charging-symbolic");
        battery.status = BatteryStatus::NotCharging;
        assert_eq!(battery.level(), StatusLevel::Normal);
        assert!(!battery.is_charging());
    }
}
