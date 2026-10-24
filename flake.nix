{
  description = "Custom status bar for Hyprland compositor";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-compat.url = "https://flakehub.com/f/edolstra/flake-compat/1.tar.gz";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-compat}: 
    let
    system = "x86_64-linux";
  overlays = [ (import rust-overlay)  ];
  pkgs = import nixpkgs { inherit system overlays;  };

  rustToolchain = pkgs.rust-bin.stable.latest.minimal;

  rustPlatform = pkgs.makeRustPlatform {
    cargo = rustToolchain;
    rustc = rustToolchain;
  };
  version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
  epic-bar = rustPlatform.buildRustPackage {
    pname = "epic-bar-rs";
    version = "${version}";
    src = ./.;
    cargoLock.lockFile = ./Cargo.lock;
    # This is a GTK application, even when dependencies pull in Qt.
    dontWrapQtApps = true;

    buildInputs = with pkgs; [
      adwaita-icon-theme
      papirus-icon-theme
      dbus
      glib
      pango
      gdk-pixbuf
      graphene
      gtk4
      gtk4-layer-shell
      librsvg
      libxml2
    ];

    nativeBuildInputs = with pkgs.buildPackages; [
      pkg-config
      wrapGAppsHook4
    ];

    postInstall = ''
      mkdir -p $out/share/epic-bar-rs/themes
      cp themes/*.css $out/share/epic-bar-rs/themes/
      mkdir -p $out/share/epic-bar-rs/assets
      cp -r assets/apps $out/share/epic-bar-rs/assets/
      find "$out/share/epic-bar-rs/assets/apps" -xtype l \
        -printf 'Omitting unresolved bundled icon alias: %f\n' -delete
    '';

    preFixup = ''
      gappsWrapperArgs+=(--prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.coreutils pkgs.iw pkgs.iproute2 pkgs.grim ]})
      # Fallback for the volume module; a pactl already on PATH wins.
      gappsWrapperArgs+=(--suffix PATH : ${pkgs.lib.makeBinPath [ pkgs.pulseaudio ]})
      gappsWrapperArgs+=(--prefix XDG_DATA_DIRS : ${pkgs.adwaita-icon-theme}/share)
      gappsWrapperArgs+=(--prefix XDG_DATA_DIRS : ${pkgs.papirus-icon-theme}/share)
    '';

  };
  in {
    packages.${system}.default = epic-bar;
    homeManagerModules.default = import ./nix/home-manager.nix self;
    homeModules.default = self.homeManagerModules.default;
    devShells.${system}.default = pkgs.mkShell {
      buildInputs = with pkgs; [
        rustToolchain
        grim
        epic-bar  
      ] ++ epic-bar.buildInputs;  

      nativeBuildInputs = epic-bar.nativeBuildInputs;
    };
  };
}
