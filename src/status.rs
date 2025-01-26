use std::{
    fs::File,
    io::Read,
    fmt,
};

use time::{OffsetDateTime,format_description};

const BATTERY_PERCENTAGE: &str = "/sys/class/power_supply/BAT1/capacity";
const BATTERY_STATUS: &str = "/sys/class/power_supply/BAT1/status";

pub struct DateTime {
    pub date: String,
    pub time: String
}

pub struct Battery {
    pub capacity: u32,
    pub icon: String,
    pub remaining: String
}

fn get_battery() -> u32 {
    let file = File::open(BATTERY_PERCENTAGE);
    let mut buff = String::with_capacity(3);

    match file {
        Ok(mut file_res) => {
            // trust it reads smth
            let _ = file_res.read_to_string(&mut buff);
        }

        Err(_) => {
            buff = "N/A".to_string();
        }
    }

    buff.trim_end().parse::<u32>().unwrap()
}

fn get_remaining()  {

}

fn get_icon() -> String {
    let file = File::open(BATTERY_STATUS);
    let mut buff  = String::with_capacity(16);

    match file {
        Ok(mut file_res) => {
            let _ = file_res.read_to_string(&mut buff);
        } 

        Err(_) => {
        }
    }

    let charge = get_battery();

    match buff.trim_end() {
        "Charging" =>{
            match charge {
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
        
        "Discharging" => {
            match charge {
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
        "Not charging" => {
            match charge {
                97..=100=> "󱈑".to_string(),
                _ => "󱃍".to_string()
            }
        }
        _ => "ruhro".to_string()
    }
}

pub fn get_battery_info() -> Battery {
    Battery {
        capacity: get_battery(),
        icon: get_icon(),
        remaining: "n/a".to_string()
    }
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

