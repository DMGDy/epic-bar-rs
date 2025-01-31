use std::{
    fs::File,
    io::Read,
    fmt,
};

use time::{OffsetDateTime,format_description};

const BATTERY_PERCENTAGE: &str = "/sys/class/power_supply/BAT1/capacity";
const BATTERY_STATUS: &str = "/sys/class/power_supply/BAT1/status";
const BATTERY_CHARGE: &str = "/sys/class/power_supply/BAT1/charge_now";
const BATTERY_CURRENT: &str = "/sys/class/power_supply/BAT1/current_now";

pub struct DateTime {
    pub date: String,
    pub time: String
}

#[derive(PartialEq)]
enum BatteryStatus {
    Discharging,
    Charging,
    Not_charging,
    Error
}

pub struct Battery {
    pub capacity: u32,
    pub icon: String,
    pub remaining: String,

    status: BatteryStatus
}

impl Default for Battery {
    fn default() -> Self {
        Battery {
            capacity: 0,
            icon: String::new(),
            remaining: String::new(),
            status: BatteryStatus::Error
        }
    }
}

pub struct Memory {
}

fn get_battery() -> u32 {
    let mut file = File::open(BATTERY_PERCENTAGE).unwrap();
    let mut buff = String::with_capacity(3);

    let _ = file.read_to_string(&mut buff);

    buff.trim_end().parse::<u32>().unwrap()
}

fn get_status() -> BatteryStatus {
    let mut file = File::open(BATTERY_STATUS).unwrap();
    let mut buff  = String::with_capacity(16);

    let _ = file.read_to_string(&mut buff);

    match buff.trim_end() {
        "Charging" => BatteryStatus::Charging,
        "Discharging" => BatteryStatus::Discharging,
        "Not charging"=> BatteryStatus::Not_charging,
        _ => BatteryStatus::Error
    }
        
}

fn get_remaining(b: &Battery)  -> String{
    // read current charge
    if b.status == BatteryStatus::Not_charging {
        return "Plugged In, not Charging".to_string();
    }
    let mut file = File::open(BATTERY_CHARGE).unwrap();
    let mut buff = String::with_capacity(32);

    let _ = file.read_to_string(&mut buff);

    let Q = buff.trim_end().parse::<f64>().unwrap();

    buff.clear();
    // get current_now
    let mut file2 = File::open(BATTERY_CURRENT).unwrap();
    let _ = file2.read_to_string(&mut buff);

    let I = buff.trim_end().parse::<f64>().unwrap();

    // time remaining in seconds
    let t: f64 =  (Q * 3600f64) / I;
    let t_h: f64 = t / 3600f64;
    let t_m: f64 = (t % 3600f64)/60f64;

    format!("{t_h} H {t_m} m remainign ").to_string()
       
}

fn get_icon(b: &Battery) -> String {

    let capacity = b.capacity;
    let status = &b.status;

    match status {
        BatteryStatus::Charging =>{
            match capacity {
                0..=10=> "󰢟".to_string(),
                11..=19=>"󰢜".to_string(),
                20..=29=>"󰂆".to_string(),
                30..=39=>"󰂇".to_string(),
                40..=49=>"󰂈".to_string(),
                50..=59=>"󰢝".to_string(),
                60..=69=>"󰂉".to_string(),
                70..=79=>"󰢞".to_string(),
                80..=89=>"󰂊".to_string(),
                90..=94=>"󰂋".to_string(),
                95..=100=>"󰂅".to_string(),
                _ => "󰂑".to_string()
            }
        },
        
        BatteryStatus::Discharging => {
            match capacity {
                0..=10=> "󱃍".to_string(),
                11..=19=>"󰁺".to_string(),
                20..=29=>"󰁻".to_string(),
                30..=39=>"󰁼".to_string(),
                40..=49=>"󰁽".to_string(),
                50..=59=>"󰁾".to_string(),
                60..=69=>"󰁿".to_string(),
                70..=79=>"󰂀".to_string(),
                80..=89=>"󰂁".to_string(),
                90..=94=>"󰂂".to_string(),
                95..=100=>"󰁹".to_string(),
                _ => "󰂑".to_string()
            }
        },
        BatteryStatus::Not_charging => {
            match capacity {
                97..=100=> "󱈑".to_string(),
                _ => "󱃍".to_string()
            }
        }
        _ => "ruhro".to_string()
    }
}

pub fn get_battery_info() -> Battery {
    let mut battery = Battery::default();
    battery.capacity = get_battery();
    battery.status = get_status();
    battery.icon = get_icon(&battery);

    battery.remaining = get_remaining(&battery);

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

// so can be easily printed out
impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f,"{}\n{}",self.date,self.time)
    }
}
