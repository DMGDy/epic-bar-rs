use std::{
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    thread,
    time::Duration,
};

use gtk::glib;

use crate::{hub::Hub, status, util};

/* GPU load, memory and temperature: NVIDIA through `nvidia-smi`, AMD through
 * amdgpu's sysfs files. Other GPUs report nothing and the module hides. */

#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub name: String,
    pub usage: Option<f64>,
    /* (used, total) in MiB. */
    pub memory: Option<(u64, u64)>,
    /* "VRAM", or "GTT" for APUs whose memory is mostly shared system RAM. */
    pub memory_kind: &'static str,
    pub temperature: Option<f64>,
}

impl Sample {
    pub fn detail(&self) -> String {
        let mut parts = vec![match self.usage {
            Some(usage) => format!("Usage {usage:.0}%"),
            None => "Usage N/A".into(),
        }];
        if let Some((used, total)) = self.memory {
            parts.push(format!(
                "{} {:.1}/{:.1} GiB",
                self.memory_kind,
                used as f64 / 1024.0,
                total as f64 / 1024.0
            ));
        }
        if let Some(temperature) = self.temperature {
            parts.push(format!("{temperature:.0} °C"));
        }
        format!("{}\n{}", self.name, parts.join(" - "))
    }
}

/* None: no supported GPU, so the module stays hidden. */
pub type Snapshot = Option<Result<Sample, String>>;

#[derive(Clone, Debug, PartialEq)]
enum Source {
    Nvidia,
    Amd(PathBuf),
}

fn parse_nvidia(text: &str) -> Result<Sample, String> {
    let line = text.lines().next().ok_or("empty nvidia-smi reply")?;
    let fields: Vec<_> = line.split(',').map(str::trim).collect();
    if fields.len() != 5 {
        return Err(format!("unexpected nvidia-smi reply: {line:?}"));
    }
    // nvidia-smi prints "[N/A]" or "[Not Supported]" for missing values.
    let number = |value: &str| value.parse::<f64>().ok();
    Ok(Sample {
        name: fields[0].to_string(),
        usage: number(fields[1]),
        memory: number(fields[2])
            .zip(number(fields[3]))
            .map(|(used, total)| (used as u64, total as u64)),
        memory_kind: "VRAM",
        temperature: number(fields[4]),
    })
}

fn read_nvidia() -> Result<Sample, String> {
    parse_nvidia(&util::output(
        "nvidia-smi",
        &[
            "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu",
            "--format=csv,noheader,nounits",
        ],
    )?)
}

fn read_number(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

const MIB: u64 = 1024 * 1024;

fn read_amd(device: &Path) -> Result<Sample, String> {
    let usage = read_number(&device.join("gpu_busy_percent"))
        .ok_or_else(|| format!("cannot read {}/gpu_busy_percent", device.display()))?;
    let memory = |kind: &str| {
        read_number(&device.join(format!("mem_info_{kind}_used")))
            .zip(read_number(&device.join(format!("mem_info_{kind}_total"))))
            .map(|(used, total)| (used / MIB, total / MIB))
    };
    // APUs carve out a small VRAM range and share the rest through GTT.
    let vram = memory("vram");
    let (memory, memory_kind) = match (vram, memory("gtt")) {
        (Some((_, total)), Some(gtt)) if total < 2048 => (Some(gtt), "GTT"),
        (vram, _) => (vram, "VRAM"),
    };
    let temperature = fs::read_dir(device.join("hwmon"))
        .ok()
        .and_then(|mut entries| entries.next())
        .and_then(Result::ok)
        .and_then(|entry| status::read_millidegrees(&entry.path().join("temp1_input")));
    let name = fs::read_to_string(device.join("product_name"))
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "AMD GPU".into());
    Ok(Sample {
        name,
        usage: Some(usage as f64),
        memory,
        memory_kind,
        temperature,
    })
}

fn find_amd(root: &Path) -> Option<PathBuf> {
    let mut cards: Vec<_> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("card") && !name.contains('-'))
        })
        .collect();
    cards.sort();
    cards
        .into_iter()
        .map(|card| card.join("device"))
        .find(|device| device.join("gpu_busy_percent").exists())
}

fn detect() -> Option<Source> {
    if util::which("nvidia-smi").is_some() && read_nvidia().is_ok() {
        return Some(Source::Nvidia);
    }
    find_amd(Path::new("/sys/class/drm")).map(Source::Amd)
}

pub fn watch(interval: Duration) -> Rc<Hub<Snapshot>> {
    let hub = Hub::new();
    let (tx, rx) = async_channel::bounded::<Snapshot>(2);
    thread::spawn(move || {
        let Some(source) = detect() else {
            let _ = tx.send_blocking(None);
            return;
        };
        loop {
            let sample = match &source {
                Source::Nvidia => read_nvidia(),
                Source::Amd(device) => read_amd(device),
            };
            if tx.send_blocking(Some(sample)).is_err() {
                return;
            }
            thread::sleep(interval);
        }
    });
    let receiver = hub.clone();
    glib::spawn_future_local(async move {
        while let Ok(snapshot) = rx.recv().await {
            if let Some(Err(error)) = &snapshot {
                eprintln!("epic-bar-rs: GPU: {error}");
            }
            receiver.publish(snapshot);
        }
    });
    hub
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_replies_tolerate_unsupported_fields() {
        let sample = parse_nvidia("NVIDIA GeForce RTX 2070 SUPER, 4, 784, 8192, 52\n").unwrap();
        assert_eq!(sample.usage, Some(4.0));
        assert_eq!(sample.memory, Some((784, 8192)));
        assert_eq!(
            sample.detail(),
            "NVIDIA GeForce RTX 2070 SUPER\nUsage 4% - VRAM 0.8/8.0 GiB - 52 °C"
        );
        let partial = parse_nvidia("Tesla, [N/A], 1, 2, [Not Supported]").unwrap();
        assert_eq!(partial.usage, None);
        assert_eq!(partial.temperature, None);
        assert!(parse_nvidia("").is_err());
        assert!(parse_nvidia("a, b").is_err());
    }

    #[test]
    fn amd_apus_report_shared_gtt_memory() {
        let root = std::env::temp_dir().join(format!("epic-bar-drm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let device = root.join("card1/device");
        fs::create_dir_all(device.join("hwmon/hwmon5")).unwrap();
        fs::create_dir_all(root.join("card1-DP-1")).unwrap();
        for (file, value) in [
            ("gpu_busy_percent", "37\n"),
            ("mem_info_vram_used", &(100 * MIB).to_string()),
            ("mem_info_vram_total", &(512 * MIB).to_string()),
            ("mem_info_gtt_used", &(2048 * MIB).to_string()),
            ("mem_info_gtt_total", &(102_400 * MIB).to_string()),
            ("hwmon/hwmon5/temp1_input", "48000"),
        ] {
            fs::write(device.join(file), value).unwrap();
        }
        assert_eq!(find_amd(&root), Some(device.clone()));
        let sample = read_amd(&device).unwrap();
        assert_eq!(sample.usage, Some(37.0));
        assert_eq!(sample.memory, Some((2048, 102_400)));
        assert_eq!(sample.memory_kind, "GTT");
        assert_eq!(sample.temperature, Some(48.0));
        assert_eq!(sample.name, "AMD GPU");
        fs::remove_dir_all(root).unwrap();
    }
}
