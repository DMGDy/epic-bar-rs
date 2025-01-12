[![dependency status](https://deps.rs/repo/github/DMGDy/epic-bar-rs/status.svg)](https://deps.rs/repo/github/DMGDy/epic-bar-rs)

# epic bar

:warning: **This is very early in development**: Currently unusable as it is being developed still !

This aims to reimplement a bar using [Eww widgets](https://github.com/elkowar/eww) that I made myself 
[here](https://github.com/DMGDy/eww-bar) as a standalone program using [GTK4](https://docs.gtk.org/gtk4/) 
and [gtk-layer-shell](https://github.com/wmww/gtk-layer-shell). 

This will only work on Hyprland as it primarily
communicates through Hyprland IPC sockets.


## why?

Most of the modules and behavior of what the bar should show has been implemented already through 
several compiled tools (although specifically for the the Eww widgets), so naturally I would like 
to implement all of it on its own. Here are the tools that generate Eww code to dynamically show widgets/
show system information:
* [workspaces](https://github.com/DMGDy/eww-workspaces)
* [statuses](https://github.com/DMGDy/statuses)
* [more statuses (in Rust)](https://github.com/DMGDy/statuses-rs)
* [Open windows in workspaces](https://github.com/DMGDy/eww-windows)

## TO-DO
Things to do in order of priority: 

1. [ ] Have workspaces visible, differentiating active status
    1. [ ] Clickable to switch to workspace
2. [ ] Show basic system information
    1. [ ] Battery status
    2. [ ] Calendar
    3. [ ] WiFi + WiFi Strength/SSID
    4. [ ] RAM + CPU
    5. [ ] Volume
3. [ ] Clickeable icon (Functionality TBD)
4. [ ] Show open windows across different workspaces
    1. [ ] With Icons
5. [ ] Easy to change colors and fonts with form of config
