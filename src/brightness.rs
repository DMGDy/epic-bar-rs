use std::{
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gio, glib, prelude::*, Align, Box, Label, Orientation};

use crate::{hub::Hub, util};

/* Laptop backlight brightness from /sys/class/backlight, changed through
 * logind's Session.SetBrightness so no udev rule or setuid helper is needed. */

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Backlight {
    pub name: String,
    path: PathBuf,
}

/* Vendor (firmware-specific) interfaces are less reliable than the GPU's
 * own; prefer raw/platform devices the way systemd-backlight does. */
fn rank(path: &Path) -> u8 {
    match fs::read_to_string(path.join("type"))
        .as_deref()
        .map(str::trim)
    {
        Ok("raw") => 0,
        Ok("platform") => 1,
        Ok("firmware") => 2,
        _ => 3,
    }
}

pub fn find(root: &Path) -> Option<Backlight> {
    let mut devices: Vec<_> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("max_brightness").exists())
        .collect();
    devices.sort_by_key(|path| (rank(path), path.clone()));
    let path = devices.into_iter().next()?;
    Some(Backlight {
        name: path.file_name()?.to_string_lossy().into_owned(),
        path,
    })
}

fn read_number(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

impl Backlight {
    fn max(&self) -> Option<u64> {
        read_number(&self.path.join("max_brightness")).filter(|max| *max > 0)
    }

    pub fn percent(&self) -> Option<u32> {
        let max = self.max()?;
        let current = read_number(&self.path.join("brightness"))?;
        Some(((current as f64 / max as f64) * 100.0).round() as u32)
    }

    /* Raw value for a percentage; never fully dark so the screen stays usable. */
    pub fn raw(&self, percent: u32) -> Option<u32> {
        let max = self.max()?;
        let raw = (max as f64 * f64::from(percent.clamp(1, 100)) / 100.0).round() as u64;
        u32::try_from(raw.max(1)).ok()
    }

    pub fn set(&self, percent: u32) {
        let Some(value) = self.raw(percent) else {
            return;
        };
        let name = self.name.clone();
        glib::spawn_future_local(async move {
            let result = async {
                let bus = gio::bus_get_future(gio::BusType::System).await?;
                bus.call_future(
                    Some("org.freedesktop.login1"),
                    "/org/freedesktop/login1/session/auto",
                    "org.freedesktop.login1.Session",
                    "SetBrightness",
                    Some(&("backlight", name.as_str(), value).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                )
                .await
            }
            .await;
            if let Err(error) = result {
                eprintln!("epic-bar-rs: cannot set brightness of {name}: {error}");
            }
        });
    }
}

/* Published by the status sampler: None without a backlight. */
pub type Snapshot = Option<u32>;

pub fn widget(backlight: Backlight, hub: &Rc<Hub<Snapshot>>, step: u32) -> Box {
    let group = Box::builder()
        .orientation(Orientation::Horizontal)
        .css_classes(["status-group", "status-brightness"])
        .valign(Align::Center)
        .spacing(4)
        .visible(false)
        .build();
    group.append(&crate::status_icon("display-brightness-symbolic"));
    let label: Label = crate::metric_label("brightness-label");
    group.append(&label);
    let current = Rc::new(std::cell::Cell::new(None::<u32>));
    let scroll_current = current.clone();
    let name = backlight.name.clone();
    util::scroll_steps(&group, move |direction| {
        if let Some(percent) = scroll_current.get() {
            let next = if direction > 0 {
                (percent + step).min(100)
            } else {
                percent.saturating_sub(step).max(1)
            };
            scroll_current.set(Some(next));
            backlight.set(next);
        }
    });
    let weak = group.downgrade();
    hub.subscribe(move |percent| {
        let Some(group) = weak.upgrade() else {
            return false;
        };
        current.set(*percent);
        group.set_visible(percent.is_some());
        if let Some(percent) = percent {
            label.set_text(&format!("{percent}%"));
            group.set_tooltip_text(Some(&format!(
                "Brightness ({name}): {percent}%\nScroll to change"
            )));
        }
        true
    });
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_raw_backlights_and_never_turns_fully_dark() {
        let root = std::env::temp_dir().join(format!("epic-bar-backlight-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (name, kind, max, current) in [
            ("acpi_video0", "firmware", "15", "15"),
            ("amdgpu_bl1", "raw", "255", "64"),
        ] {
            let path = root.join(name);
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("type"), kind).unwrap();
            fs::write(path.join("max_brightness"), max).unwrap();
            fs::write(path.join("brightness"), current).unwrap();
        }
        let backlight = find(&root).unwrap();
        assert_eq!(backlight.name, "amdgpu_bl1");
        assert_eq!(backlight.percent(), Some(25));
        assert_eq!(backlight.raw(0), Some(3));
        assert_eq!(backlight.raw(100), Some(255));
        assert!(find(&root.join("missing")).is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
