use std::{
    fs::File,
    io::{Write,Read},
};

const BATTERY_PERCENTAGE: &str = "/sys/class/power_supply/BAT1/capacity";

pub fn get_battery() -> String {
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
    buff.trim_end().to_string()
}
