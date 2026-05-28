// Windows 11 Style Theme - Light Mode
pub const CSS: &str = "
/* ============================================
   WINDOWS 11 STYLE THEME - LIGHT
   ============================================ */

/* Window base */
window {
    background-color: #f3f3f3;
}

button {
    font-family: 'Segoe UI Variable', 'Segoe UI', 'SF Pro', sans-serif;
    border-radius: 0px;
    margin: 0px;
    padding: 5px 10px;
    color: #1a1a1a;
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
    text-shadow: none;
    -gtk-icon-shadow: none;
    transition: all 100ms ease;
}

button label {
    color: #1a1a1a;
}

button:hover {
    background-color: rgba(0,0,0,0.06);
    background-image: none;
}

label {
    color: #1a1a1a;
    font-family: 'Segoe UI Variable', 'Segoe UI', 'SF Pro', sans-serif;
}

box {
    font-family: 'Segoe UI Variable', 'Segoe UI', 'SF Pro', sans-serif;
    color: #1a1a1a;
}

/* ============================================
   TOP BAR - Windows 11 Style
   ============================================ */
top-bar {
    background-color: #f3f3f3;
    padding: 0px;
}

/* Status reveal button */
status-reveal-button {
    font-size: 20px;
    padding: 5px 10px;
    margin: 0px;
    color: #1a1a1a;
}

/* Battery styling */
battery-icon {
    padding: 5px 5px;
    font-size: 18px;
    color: #1a1a1a;
}

battery-label {
    font-size: 15px;
    padding: 5px 10px 5px 5px;
    color: rgba(0,0,0,0.8);
}

/* Memory styling */
mem-icon {
    padding: 5px 5px;
    font-size: 18px;
    color: #1a1a1a;
}

mem-label {
    font-size: 15px;
    padding: 5px 10px 5px 5px;
    color: rgba(0,0,0,0.8);
}

/* CPU styling */
cpu-label {
    font-size: 15px;
    padding: 5px 10px 5px 5px;
    color: rgba(0,0,0,0.8);
}

cpu-load-label {
    font-size: 15px;
    padding: 5px 5px;
    color: rgba(0,0,0,0.8);
}

/* ============================================
   WORKSPACE BUTTONS - Windows 11 Style
   ============================================ */

/* Occupied workspace */
.occupied {
    background-color: transparent;
    background-image: none;
    color: #505050;
    border-radius: 0px;
    border: none;
    margin: 0px;
    padding: 5px 13px;
    transition: all 100ms ease;
    -gtk-icon-style: symbolic;
}

.occupied label {
    color: #505050;
}

.occupied:hover {
    background-color: rgba(0,0,0,0.06);
    background-image: none;
    color: #1a1a1a;
}

.occupied:hover label {
    color: #1a1a1a;
}

/* Active workspace - Windows 11 accent with glow */
.active {
    background-color: rgba(37,99,235,0.16);
    background-image: none;
    color: #1a1a1a;
    border-radius: 0px;
    border: none;
    margin: 0px;
    padding: 5px 13px;
    box-shadow: 0 0 10px rgba(37,99,235,0.4);
    transition: all 100ms ease;
}

.active label {
    color: #1a1a1a;
}

.active:hover {
    background-color: rgba(37,99,235,0.24);
    background-image: none;
    box-shadow: 0 0 15px rgba(37,99,235,0.5);
}

/* Empty active workspace */
.empty-active {
    background-color: transparent;
    background-image: none;
    color: #707070;
    border-radius: 0px;
    border: none;
    margin: 0px;
    padding: 5px 13px;
}

.empty-active label {
    color: #707070;
}

/* ============================================
   DATE/TIME - Windows 11 Style
   ============================================ */
date-container {
    font-size: 15px;
    padding: 5px 13px;
    color: #1a1a1a;
    border-radius: 0px;
}

date-container:hover {
    background-color: rgba(0,0,0,0.06);
}

/* ============================================
   BOTTOM BAR - Windows 11 Taskbar
   ============================================ */
bottom-bar {
    background-color: #f3f3f3;
    padding: 0px;
}

/* Workspace tag label */
tag-label {
    font-size: 18px;
    font-weight: 500;
    color: rgba(0,0,0,0.7);
    padding: 5px 10px;
    margin: 0px;
}

/* ============================================
   WINDOW BUTTONS - Windows 11 Style
   ============================================ */

/* Active window - glow effect */
active-window-box {
    background-color: rgba(37,99,235,0.12);
    color: #1a1a1a;
    font-size: 15px;
    margin: 0px;
    padding: 8px 13px;
    border-radius: 0px;
    border: none;
    box-shadow: 0 0 15px rgba(37,99,235,0.5), 0 0 30px rgba(37,99,235,0.24);
    transition: all 100ms ease;
}

active-window-box:hover {
    background-color: rgba(37,99,235,0.2);
    box-shadow: 0 0 20px rgba(37,99,235,0.6), 0 0 40px rgba(37,99,235,0.36);
}

/* Inactive window button */
window-box {
    background-color: transparent;
    color: rgba(0,0,0,0.7);
    font-size: 15px;
    margin: 0px;
    padding: 8px 13px;
    border-radius: 0px;
    border: none;
    transition: all 100ms ease;
}

window-box:hover {
    background-color: rgba(0,0,0,0.06);
    color: #1a1a1a;
}

/* Empty window indicator */
window-box-empty {
    color: rgba(0,0,0,0.3);
    font-size: 15px;
    padding: 0px 3px;
}

/* Icon spacing and background */
icon-image {
    padding: 6px;
    margin-right: 3px;
    background-color: rgba(0,0,0,0.06);
    border-radius: 0px;
}

/* ============================================
   WORKSPACE WINDOW CONTAINERS
   ============================================ */

/* Empty workspace container */
workspace-window-box-empty {
    margin: 0px;
    padding: 0px;
    border-radius: 0px;
    border: none;
    background-color: transparent;
}

/* Active workspace container */
workspace-window-box-active {
    margin: 0px;
    padding: 0px;
    border-radius: 0px;
    border: none;
    background-color: rgba(37,99,235,0.06);
}

/* Normal workspace container */
workspace-window-box {
    margin: 0px;
    padding: 0px;
    border-radius: 0px;
    border: none;
    background-color: transparent;
}

";
