use std::{
    env, io,
    path::PathBuf,
    process::{Command, Stdio},
};

use gtk::{gio, prelude::*};

/* Find `program` on PATH. */
pub fn which(program: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}

/* Run a short-lived helper (pactl, dunstctl, nvidia-smi...) with a hard
 * timeout and return its stdout. Never call this on the GTK main thread. */
pub fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("timeout")
        .args(["--kill-after=1s", "3s", program])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{program} {} exited with {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/* Run a helper for its side effect off the main thread, logging failures. */
pub fn run_detached(program: &'static str, args: Vec<String>) {
    gio::spawn_blocking(move || {
        let args: Vec<_> = args.iter().map(String::as_str).collect();
        if let Err(error) = output(program, &args) {
            eprintln!("epic-bar-rs: {error}");
        }
    });
}

/* Start a user-configured shell command without waiting for it. */
pub fn spawn_shell(command: &str) -> io::Result<()> {
    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::null())
        .spawn()?;
    // Reap the child in the background so it does not linger as a zombie.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/* A scroll controller reporting -1/+1 steps (up = +1), for volume and
 * brightness widgets. Smooth-scrolling touchpads are accumulated. */
pub fn scroll_steps(widget: &impl IsA<gtk::Widget>, step: impl Fn(i32) + 'static) {
    let scroll = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
    );
    scroll.connect_scroll(move |_, _, dy| {
        if dy < 0.0 {
            step(1);
        } else if dy > 0.0 {
            step(-1);
        }
        gtk::glib::Propagation::Stop
    });
    widget.add_controller(scroll);
}

/* Call `click` for presses of a specific mouse button (2 = middle, 3 = right). */
pub fn on_button(widget: &impl IsA<gtk::Widget>, button: u32, click: impl Fn() + 'static) {
    let gesture = gtk::GestureClick::builder().button(button).build();
    gesture.connect_pressed(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        click();
    });
    widget.add_controller(gesture);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_report_missing_programs_and_failures() {
        assert!(which("sh").is_some());
        assert!(which("epic-bar-definitely-missing").is_none());
        assert_eq!(output("echo", &["hi"]).unwrap(), "hi\n");
        assert!(output("false", &[]).is_err());
    }
}
