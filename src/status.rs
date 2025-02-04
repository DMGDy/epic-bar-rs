use std::{
    fs::File,
    io::Read,
    fmt,
    string,
};

use time::{OffsetDateTime,format_description};


const BATTERY_PERCENTAGE: &str = "/sys/class/power_supply/BAT1/capacity";
const BATTERY_STATUS: &str = "/sys/class/power_supply/BAT1/status";
const BATTERY_CHARGE: &str = "/sys/class/power_supply/BAT1/charge_now";
const BATTERY_CHARGE_FULL: &str = "/sys/class/power_supply/BAT1/charge_full";
const BATTERY_CURRENT: &str = "/sys/class/power_supply/BAT1/current_now";

const MEMORY_INFO: &str = "/proc/meminfo";

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
    pub fraction: f64,
    pub string: String
}

fn get_battery(b: &mut Battery) {
    let mut file = File::open(BATTERY_PERCENTAGE).unwrap();
    let mut buff = String::with_capacity(3);

    let _ = file.read_to_string(&mut buff);

    b.capacity = buff.trim_end().parse::<u32>().unwrap();
}

fn get_status(b: &mut Battery) {
    let mut file = File::open(BATTERY_STATUS).unwrap();
    let mut buff  = String::with_capacity(16);

    let _ = file.read_to_string(&mut buff);

    b.status = match buff.trim_end() {
        "Charging" => BatteryStatus::Charging,
        "Discharging" => BatteryStatus::Discharging,
        "Not charging"=> BatteryStatus::NotCharging,
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
            let mut file = File::open(BATTERY_CHARGE).unwrap();
            let mut buff = String::with_capacity(32);
            let _ = file.read_to_string(&mut buff);
            let charge_now = buff.trim_end().parse::<f64>().unwrap();

            buff.clear();

            // get current_now
            let mut file2 = File::open(BATTERY_CURRENT).unwrap();
            let _ = file2.read_to_string(&mut buff);
            let current = buff.trim_end().parse::<f64>().unwrap();

            match state {
                BatteryStatus::Charging =>{
                    buff.clear();
                    // get full charge of battery to find remaining charge
                    let _ = File::open(BATTERY_CHARGE_FULL)
                        .unwrap()
                        .read_to_string(&mut buff);
                    let charge_remaining = buff.trim_end()
                        .parse::<f64>()
                        .unwrap() - charge_now;
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
    let dt = OffsetDateTime::now_local().unwrap();
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
    let mut file = File::open(MEMORY_INFO).unwrap();
    let mut buff = [0;96]; // no need to read entire file
    file.read_exact(&mut buff).unwrap();

    let binding =  String::from_utf8_lossy(&buff);
    let mut string = binding.lines();

    let memtotal_kb: f64 = string.nth(0).unwrap()
        .to_string()
        .split_whitespace()
        .nth(1).unwrap()
        .parse().unwrap();

    let memused_kb: f64 = string.nth(1).unwrap()
        .to_string()
        .split_whitespace()
        .nth(1).unwrap()
        .parse().unwrap();

    let total = memtotal_kb / 1_048_576.0;
    let used = (memtotal_kb - memused_kb)/1_048_576.0;

    let string = format!("{:.1}/{:.1} GiB",used,total).to_string();
    let fraction: f64 = used/total;

    Memory {
        fraction,
        string
    }
}
