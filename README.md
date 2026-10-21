[![dependency status](https://deps.rs/repo/github/DMGDy/epic-bar-rs/status.svg)](https://deps.rs/repo/github/DMGDy/epic-bar-rs)

# epic bar

:warning: **This is in development**: Currently the status bar I use every day

![First progress image](sample/update2.png "Current State")


This aims to reimplement a bar using [Eww widgets](https://github.com/elkowar/eww) that I made myself 
[here](https://github.com/DMGDy/eww-bar) as a standalone program using [GTK4](https://docs.gtk.org/gtk4/) 
and [gtk-layer-shell](https://github.com/wmww/gtk-layer-shell). 

This will only work on Hyprland as it primarily
communicates through Hyprland IPC sockets.

Additionally this uses ![Candy icons](https://github.com/EliverLara/candy-icons) and ![Kora](https://github.com/bikass/kora) for the images used in the bar (application icons,battery,RAM).

All content is licensed under GPL-3.

## Trying it out
If you are on NixOS or using Nix on your system and using Hyprland you can try it out without cloning the repository
by simply running the following if flake feature is enabled:
```
nix run github:DMGDy/epic-bar-rs
```
If flake is not enabled, clone repo and then:
```
nix-build
./result/bin/epic-bar-rs
```

### Modifying and Rebuilding
If you wanted to make changes to the source and rebuild with flake you can enter a development environment with:
(note: both will run the Cargo project build)
```
nix develop
```
or without flake:
```
nix-shell
```

## Themes

Themes are GTK CSS files, loaded once at startup for both bars. Editing a theme
does not require recompiling; restart the bar to apply changes.

The lookup order is:

1. `EPIC_BAR_THEME`, when set (an explicit missing or invalid file is an error).
2. `$XDG_CONFIG_HOME/epic-bar-rs/theme.css`, or `~/.config/epic-bar-rs/theme.css`.
3. The installed `share/epic-bar-rs/themes/autumn-light.css`.
4. `theme.css` in the current directory, for development.

`theme.css` selects the high-contrast **Autumn Light** theme using a relative
CSS import. It matches the warm white, saturated red, and cocoa colors in the
Kitty/tmux palette. The former embedded stylesheet is preserved unchanged in
`themes/windows-11-light.css`. The Autumn theme imports that stylesheet to keep
its original font stack and refines its spacing, colors, and active indicators.

### Status-bar experiment

This branch merges `event-driven-updates`: one persistent, reconnecting Hyprland
event stream updates both bars from the same JSON workspace snapshot, coalescing
event bursts instead of polling each bar separately. Runtime CPU-core detection
and panic-free status reads are retained alongside the Autumn refinements.
The current Lua dispatch syntax and reading dispatch replies are preserved.

The `experiment/autumn-status-bars` branch keeps the warm-white background opaque
and replaces active-item glows with a peach fill and a 2 px red underline.
The top and bottom bars target 48 px and 38 px respectively, with 16 px text,
4 px workspace/status gaps, 12 px bottom workspace-group gaps, and 18 px icons.
Top workspace buttons are full-height 47 px squares, flush with the top, bottom
separator, and left edge of the bar without rounded-corner gaps. Bottom task labels and
workspace tags use the same 16 px text size as the top bar.
Hovering a top workspace lists its current window titles in recent-use order;
empty workspaces show "No open windows". The list refreshes with window opens,
closes, title changes, and workspace moves. Titles are plain text, not markup.
The active top-workspace highlight slides between buttons over 200 ms with
ease-out motion, continuing smoothly if another workspace is selected mid-slide.
It follows real button positions, stops requesting frames when settled, and
respects GTK's disabled-animation setting. The highlight reads the existing
Autumn palette from runtime CSS; the saved Windows theme keeps its static highlight.

The top bar contains workspaces, active network devices, compact CPU/memory/battery
percentages, and a two-row clock with the date (`YYYY/MM/DD`) above the time
(`HH:MM:SS`), preceded by a symbolic calendar icon. Both rows are right-aligned;
the taller top bar keeps them unclipped.
The more button toggles CPU frequency and used/total memory;
full memory values are also available in tooltips. Numeric labels
use tabular digits and natural text widths, without spare character slots or
leading icon-to-value whitespace. Widths may change when the digit count changes;
the fixed-format clock stays stable. The memory and calendar groups have a
compact 4 px inter-group gap.
System icons remain symbolic but use brighter colors independent of the Autumn
palette: blue CPU, purple memory, cyan wired links, green Wi-Fi and battery,
teal charging, pink calendar, and an orange more button. Checking/offline links
are slate; warning icons turn amber and critical icons red. Metric text and
alert backgrounds keep their existing Autumn colors. Application icons retain
their original colors without individual background tiles.

Network devices follow the feature from `event-driven-updates` (`fadfb6d`,
`d85b608`): every physical interface with an up link and carrier is shown,
excluding loopback and virtual/container interfaces. The IPv4 default-route
device appears first, followed by other devices alphabetically. Wi-Fi shows its
SSID when available, with the device name in its tooltip; wired links show their
device names. Labels remain visible in the compact view, and long names are
ellipsized with full values in tooltips.

Network reads and SSID queries run off the GTK thread. Each wireless interface
has its own 10-second SSID cache; packaged `iw` queries have a timeout. Unchanged
device lists retain their widgets. No active links shows `Offline`; enumeration
failures instead show `Network error` with details in the tooltip and stderr.

Normal metrics use secondary brown. CPU usage warns at 85% and becomes critical
at 95%; memory usage uses 80%/90%. A discharging battery warns at 20% and becomes
critical at 10%. Warning/error icons accompany amber/red backgrounds, so alerts
do not rely on color alone. Charging batteries do not trigger low-charge alerts.

The bottom bar remains the task list. Buttons are retained by Hyprland window
address: focus changes animate windows and workspace tags into their existing
most-recently-used order over 200 ms, rather than rebuilding the entire strip.
Rapid switches retarget from the visible positions; disabled GTK animations snap
directly to the new order. Titles, icons, tooltips, and click handlers stay with
their window when it changes position or workspace.
Buttons reserve their measured icon/title width, remeasured as titles grow or
shrink. Only long titles are ellipsized, with full window information in tooltips.
Overflow scrolls horizontally using the wheel
or trackpad, without adding a scrollbar or growing the panel.
All Autumn tooltips use an explicit warm-white background, dark cocoa text,
a subtle border, and 16 px text, independent of GTK's light/dark preference.

Hyprland window-attention requests highlight the affected task button, its
bottom workspace tag, and its top workspace square in amber. The highlight
pulses twice over 1.2 seconds, then stays solid until that window receives
focus or closes. GTK's disabled-animation setting uses a solid highlight
immediately. The normal red active-workspace underline remains visible even
when another window on that workspace needs attention.
Window and workspace ordering do not change when attention arrives.
Visiting a workspace only clears attention for the window that actually gains
focus; other urgent windows keep their indicators. Highlights follow windows
when they move, and tooltips identify windows that need attention.

Attention uses the existing Hyprland event connection, including ordered
per-window urgency/focus events when refreshes are coalesced, without polling.
It is not a desktop-notification listener: apps must request window attention;
a Dunst popup alone does not necessarily do so. The pinned Hyprland version
does not include per-window urgency in its JSON snapshots, so attention is
learned while the bar is connected, not recovered after a restart or for
events missed during a connection outage.

To select the saved theme, change `theme.css` to:

```css
@import url("themes/windows-11-light.css");
```

Alternatively, choose a file for one launch:

```sh
EPIC_BAR_THEME="$PWD/themes/windows-11-light.css" ./target/release/epic-bar-rs
```

Relative imports resolve against the CSS file's directory. Unreadable files,
invalid CSS, and missing imports are reported on stderr and stop startup rather
than silently falling back to another theme.

## why?

Most of the modules and behavior of what the bar should show has been implemented already through 
several compiled tools (although specifically for the the Eww widgets), so naturally I would like 
to implement all of it on its own. Here are the tools that generate Eww code to dynamically show widgets/
show system information:

* [workspaces](https://github.com/DMGDy/eww-workspaces)
* [statuses](https://github.com/DMGDy/statuses)
* [more statuses (in Rust)](https://github.com/DMGDy/statuses-rs)
* [Open windows in workspaces](https://github.com/DMGDy/eww-windows)

I previously relied on Rust crates to obtain information such as Netowrk and Wi-fi information but 
~~I will plan on simply using D-bus to interact with these services.~~
Interacting with Netlink IPC socket to get network information would be much more direct
and fast.

## TO-DO
Things to do in order of priority: 

1. [X] Have workspaces visible, differentiating active status
    1. [X] Clickable to switch to workspace
2. [ ] Show basic system information
    1. [X] Battery status
    2. [X] Calendar
    3. [ ] WiFi + WiFi Strength/SSID
    4. [X] RAM 
    5. [X] CPU
        1. [X] Average load through `/stat/proc`
        2. [X] Frequency accross CPUs averaged from `/sys/devices/system/cpu/cpuX/cpufreq/scaling_cur_freq`
    6. [ ] Volume (maybe most difficult)
3. [ ] Clickeable icon (Functionality TBD)
4. [X] Show open windows across different workspaces (Bottom bar or integrated in seperator of top bar)
    1. [X] With Icons
5. [X] Change colors and fonts using runtime CSS theme files
6. [ ] Stop using `.unwrap()` and implement propper error handling (default values, fallback behavior/values)

## Known Issues
* Only expects and appears on a single display
* If display is turned off and on, bar is no longer present but process still running
* ghostty has no current icons
    * need to properly create submodules of assets rather than coppying the files...
