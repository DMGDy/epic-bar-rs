# Home Manager module: `programs.epic-bar`.
#
#   imports = [ inputs.epic-bar-rs.homeManagerModules.default ];
#   programs.epic-bar = {
#     enable = true;
#     theme = "autumn-dark";
#     settings = {
#       workspaces = 10;
#       outputs."HDMI-A-1".workspaces = [ 4 5 6 ];
#     };
#   };
#
# The theme and config are generated into the Nix store and passed to the
# bar through EPIC_BAR_THEME / EPIC_BAR_CONFIG, so nothing is written to
# ~/.config/epic-bar-rs (which may be a development checkout).
self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.epic-bar;
  toml = pkgs.formats.toml { };
  builtinThemes = [ "autumn-light" "autumn-dark" "windows-11-light" ];
  themeSource =
    if builtins.isString cfg.theme && builtins.elem cfg.theme builtinThemes then
      "${cfg.package}/share/epic-bar-rs/themes/${cfg.theme}.css"
    else
      "${cfg.theme}";
  themeFile = pkgs.writeText "epic-bar-theme.css" ''
    @import url("${themeSource}");
    ${cfg.extraCss}
  '';
  configFile = toml.generate "epic-bar-config.toml" cfg.settings;
in
{
  options.programs.epic-bar = {
    enable = lib.mkEnableOption "epic-bar-rs, a GTK4 status bar for Hyprland";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "epic-bar-rs.packages.\${system}.default";
      description = "The epic-bar-rs package to run.";
    };

    theme = lib.mkOption {
      type = lib.types.either (lib.types.enum builtinThemes) lib.types.path;
      default = "autumn-light";
      example = "autumn-dark";
      description = ''
        A bundled theme name, or a path to a GTK CSS file. A custom file can
        build on a bundled one with
        `@import url("''${package}/share/epic-bar-rs/themes/autumn-dark.css");`.
      '';
    };

    extraCss = lib.mkOption {
      type = lib.types.lines;
      default = "";
      example = ''
        .status-cpu .status-icon { color: #e0675a; }
      '';
      description = "GTK CSS appended after the theme.";
    };

    settings = lib.mkOption {
      type = toml.type;
      default = { };
      example = lib.literalExpression ''
        {
          workspaces = 10;
          bar.right = [ "tray" "network" "cpu" "memory" "volume" "clock" "power" ];
          outputs."HDMI-A-1" = { workspaces = [ 4 5 6 ]; tasks = false; };
          clock.time = "[hour]:[minute]";
          power.lock = "hyprlock";
        }
      '';
      description = ''
        Bar configuration, written as TOML. See the README's Configuration
        section for every option; unknown keys are rejected at startup.
      '';
    };

    systemd = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Run the bar as a systemd user service.";
      };

      target = lib.mkOption {
        type = lib.types.str;
        default = config.wayland.systemd.target or "graphical-session.target";
        defaultText = lib.literalExpression ''config.wayland.systemd.target'';
        description = ''
          The systemd user target that starts and stops the bar. It needs
          WAYLAND_DISPLAY and HYPRLAND_INSTANCE_SIGNATURE in the systemd user
          environment, which Home Manager's Hyprland module exports by default.
        '';
      };
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    # For running the bar by hand with the same theme and settings.
    home.sessionVariables = {
      EPIC_BAR_THEME = "${themeFile}";
      EPIC_BAR_CONFIG = "${configFile}";
    };

    systemd.user.services.epic-bar = lib.mkIf cfg.systemd.enable {
      Unit = {
        Description = "epic-bar-rs status bar";
        Documentation = "https://github.com/DMGDy/epic-bar-rs";
        PartOf = [ cfg.systemd.target ];
        After = [ cfg.systemd.target ];
        ConditionEnvironment = "WAYLAND_DISPLAY";
        X-Restart-Triggers = [ "${themeFile}" "${configFile}" ];
      };
      Service = {
        ExecStart = lib.getExe' cfg.package "epic-bar-rs";
        Environment = [
          "EPIC_BAR_THEME=${themeFile}"
          "EPIC_BAR_CONFIG=${configFile}"
        ];
        Restart = "on-failure";
        RestartSec = 2;
      };
      Install.WantedBy = [ cfg.systemd.target ];
    };
  };
}
