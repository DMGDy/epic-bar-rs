use std::{
    fs::File,
    io::Read,
    path::Path,
    thread,
    fmt,
};

use time::{OffsetDateTime,format_description};


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

pub struct DateTime {
    pub date: String,
    pub time: String
}

#[derive(PartialEq)]
enum BatteryStatus {
    Discharging,
    Charging,
    NotCharging,
    Error
}

pub struct Battery {
    pub capacity: u32, //percentage left on battery
    pub icon: String, //return path to image for icon
    pub tooltip_text: String, // formatted with remaining
    remaining: String, 

    status: BatteryStatus
}

impl Default for Battery {
    fn default() -> Self {
        Battery {
            capacity: 0,
            icon: String::new(),
            remaining: String::new(),
            tooltip_text: String::new(),
            status: BatteryStatus::Error
        }
    }
}

pub struct Memory {
    pub string: String
}

#[derive(Clone, Copy)]
pub struct Cpu {
    total_load1: u32,
    non_idle_load1: u32,
    total_load2: u32,
    non_idle_load2: u32,

    pub avg_load:f32
}

pub mod Network {
}

// read a sysfs/procfs file and return its trimmed contents, or None on any error
fn read_trimmed(path: &str) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut buff = String::new();
    file.read_to_string(&mut buff).ok()?;
    Some(buff.trim_end().to_string())
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
fn read_cpu_totals() -> Option<(u32, u32)> {
    let mut file = File::open(CPU_STAT).ok()?;
    // just need first line of aggregate CPU usage measurements
    let mut buff = [0; 64];
    file.read_exact(&mut buff).ok()?;

    let binding = String::from_utf8_lossy(&buff);
    let line = binding.lines().next()?;
    let cpu_totals = line.split_whitespace();

    let mut count = 0;
    let mut total_t = 0u32;
    let mut idle_t = 0u32;
    // cpu user nice system idle iowait irq softirq ...
    //  0   1    2     3     4     5     6     7
    // idle fields (4,5) are summed separately
    for field in cpu_totals {
        match count {
            0 => { count += 1; continue; }
            4 | 5 => {
                count += 1;
                idle_t += field.parse::<u32>().unwrap_or(0);
                continue;
            }
            8 => break,
            _ => { count += 1; }
        }
        total_t += field.parse::<u32>().unwrap_or(0);
    }
    Some((total_t + idle_t, total_t))
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
            avg_load: 0.0
        }
    }

    pub fn get_avg_freq() -> String {
        let mut handles = Vec::new();

        // each handle reads one core's current frequency in Hz (None on failure)
        for path in cpu_freq_paths() {
            let handle = thread::spawn(move || -> Option<f32> {
                let mut file = File::open(&path).ok()?;
                let mut buff = String::new();
                file.read_to_string(&mut buff).ok()?;
                buff.trim_end().parse::<f32>().ok()
            });
            handles.push(handle);
        }

        // average only over cores read successfully; divisor is the live count
        let freqs: Vec<f32> = handles
            .into_iter()
            .filter_map(|h| h.join().ok().flatten())
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

    pub fn get_cpu_load(&mut self) -> f32{
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

    pub fn get_cpu_image(&self) -> String {
        match self.avg_load {
            l if l <= 12.5 => "assets/status/indicator-cpufreq.svg",
            l if l > 12.5 && l <= 45.0 => "assets/status/indicator-cpufreq-25.svg",
            l if l > 45.0 && l <= 70.0 => "assets/status/indicator-cpufreq-50.svg",
            l if l > 70.0 && l <= 90.0 => "assets/status/indicator-cpufreq-75.svg",
            l if l > 90.0 && l<= 100.0 => "assets/status/indicator-cpufreq-100.svg",
            _ => "assets/status/indicator-cpufreq.svg"
        }.to_string()
    }

}

fn get_battery(b: &mut Battery) {
    if let Some(cap) = read_trimmed(BATTERY_PERCENTAGE)
        .and_then(|s| s.parse::<u32>().ok())
    {
        b.capacity = cap;
    }
}

fn get_status(b: &mut Battery) {
    b.status = match read_trimmed(BATTERY_STATUS).as_deref() {
        Some("Charging") => BatteryStatus::Charging,
        Some("Discharging") => BatteryStatus::Discharging,
        Some("Not charging") => BatteryStatus::NotCharging,
        _ => BatteryStatus::Error
    };
}

fn get_battery_tooltip_text(b: &mut Battery) {
    let status_text = match b.status {
        BatteryStatus::Charging => "Charging",
        BatteryStatus::Discharging => "Discharging",
        BatteryStatus::NotCharging=> "Plugged in, Not Charging",
        _ => "Error getting state"
    };

    b.tooltip_text = format!("{status_text}\n{}",b.remaining);
}

// TODO: not considering charging until full time
fn get_remaining(b: &mut Battery) {
    // read current charge
    match &b.status {
        state @ (BatteryStatus::Charging | BatteryStatus::Discharging)=> {
            let charge_now = match read_trimmed(BATTERY_CHARGE)
                .and_then(|s| s.parse::<f64>().ok())
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
                BatteryStatus::Charging =>{
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
                    let t = (charge_remaining*3600f64)/current;
                    let t_h = t/3600f64;
                    let t_m = (t%3600f64)/60f64;

                    if t_h >= 1.0 {
                        b.remaining = format!("{} hours {} minutes to full"
                            ,t_h as u32,t_m as u32).to_string();
                    } else {
                        b.remaining = format!("{} minutes to full"
                            ,t_m as u32).to_string();
                    }

                },
                BatteryStatus::Discharging => {
                    // time remaining in seconds
                    let t: f64 =  (charge_now * 3600f64) / current;
                    let t_h: f64 = t / 3600f64;
                    let t_m: f64 = (t % 3600f64)/60f64;

                    if t_h >= 1.0 {
                        b.remaining = format!("{} hours {} minutes remaining"
                            ,t_h as u32,t_m as u32).to_string();
                    } else {
                        b.remaining = format!("{} minutes remaining"
                            ,t_m as u32).to_string();
                    }
                }
                _ => {()}
            };

        },
        BatteryStatus::NotCharging => b.remaining = "Fully charged".to_string(),
        _ => b.remaining = "Error getting state".to_string(),

    };
}

fn get_icon(b: &mut Battery) {

    let capacity = b.capacity;
    let status = &b.status;

    b.icon = match status {
        BatteryStatus::Charging =>{

            match capacity {
                0..=10=> "assets/status/battery-000-charging.svg".to_string(),
                11..=19=>"assets/status/battery-010-charging.svg".to_string(),
                20..=29=>"assets/status/battery-020-charging.svg".to_string(),
                30..=39=>"assets/status/battery-030-charging.svg".to_string(),
                40..=49=>"assets/status/battery-040-charging.svg".to_string(),
                50..=59=>"assets/status/battery-050-charging.svg".to_string(),
                60..=69=>"assets/status/battery-060-charging.svg".to_string(),
                70..=79=>"assets/status/battery-070-charging.svg".to_string(),
                80..=89=>"assets/status/battery-080-charging.svg".to_string(),
                90..=94=>"assets/status/battery-090-charging.svg".to_string(),
                95..=100=>"assets/status/battery-100-charging.svg".to_string(),
                _ => "assets/status/battery-missing.svg".to_string()
            }
        },

        BatteryStatus::Discharging => {
            match capacity {
                0..=10=>  "assets/status/battery-000.svg".to_string(),
                11..=19=> "assets/status/battery-010.svg".to_string(),
                20..=29=> "assets/status/battery-020.svg".to_string(),
                30..=39=> "assets/status/battery-030.svg".to_string(),
                40..=49=> "assets/status/battery-040.svg".to_string(),
                50..=59=> "assets/status/battery-050.svg".to_string(),
                60..=69=> "assets/status/battery-060.svg".to_string(),
                70..=79=> "assets/status/battery-070.svg".to_string(),
                80..=89=> "assets/status/battery-080.svg".to_string(),
                90..=94=> "assets/status/battery-090.svg".to_string(),
                95..=100=>"assets/status/battery-100.svg".to_string(),
                _ => "assets/status/battery-missing.svg".to_string()
            }
        },

        BatteryStatus::NotCharging => {
            match capacity {
                97..=100=> "assets/status/battery-full-charging.svg".to_string(),
                _ => "assets/status/battery-missing.svg".to_string()
            }
        }
        _ => "assets/status/battery-missing.svg".to_string()
    }
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

fn get_date(dt: &OffsetDateTime) -> String {
    let date = dt.date();
    let format = format_description::parse("[year]/[month]/[day]").unwrap();
    date.format(&format).unwrap()
}

fn get_time(dt: &OffsetDateTime) -> String {
    let time = dt.time();
    let format = format_description::parse("[hour]:[minute]:[second]").unwrap();
    time.format(&format).unwrap()
}

pub fn get_datetime() -> DateTime {
    // fall back to UTC if the local offset can't be determined
    let dt = OffsetDateTime::now_local()
        .unwrap_or_else(|_| OffsetDateTime::now_utc());
    DateTime{
        date: get_date(&dt),
        time: get_time(&dt)
    }
}

// so can be easily format out
impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f,"{}\n{}",self.date,self.time)
    }
}

pub fn get_mem_info() -> Memory {
    Memory {
        string: read_mem_info().unwrap_or_else(|| "N/A".to_string())
    }
}

fn read_mem_info() -> Option<String> {
    let mut file = File::open(MEMORY_INFO).ok()?;
    let mut buff = [0;96]; // no need to read entire file
    file.read_exact(&mut buff).ok()?;

    let binding =  String::from_utf8_lossy(&buff);
    let mut lines = binding.lines();

    let memtotal_kb: f64 = lines.next()?
        .split_whitespace()
        .nth(1)?
        .parse().ok()?;

    let memused_kb: f64 = lines.nth(1)?
        .split_whitespace()
        .nth(1)?
        .parse().ok()?;

    let total = memtotal_kb / 1_048_576.0;
    let used = (memtotal_kb - memused_kb)/1_048_576.0;

    Some(format!("{:.1}/{:.1} GiB",used,total))
}
