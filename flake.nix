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
  # The older flake pin has grim 1.4, without native toplevel capture.
  preview-grim = pkgs.grim.overrideAttrs (old: {
    version = "1.5.0";
    src = pkgs.fetchFromGitLab {
      domain = "gitlab.freedesktop.org";
      owner = "emersion";
      repo = "grim";
      rev = "v1.5.0";
      hash = "sha256-oPo6zrS3gCnviIK0+gPvtal+6c7fNFWtXnAA0YfaS+U=";
    };
    depsBuildBuild = (old.depsBuildBuild or []) ++ [ pkgs.pkg-config ];
  });
  epic-bar = rustPlatform.buildRustPackage {
    pname = "epic-bar-rs";
    version = "${version}";
    src = ./.;
    cargoLock.lockFile = ./Cargo.lock;

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
      wrapGAppsHook
    ];

    postInstall = ''
      mkdir -p $out/share/epic-bar-rs/themes
      cp themes/*.css $out/share/epic-bar-rs/themes/
      mkdir -p $out/share/epic-bar-rs/assets
      cp -r assets/apps $out/share/epic-bar-rs/assets/
    '';

    preFixup = ''
      gappsWrapperArgs+=(--prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.coreutils pkgs.iw pkgs.iproute2 preview-grim ]})
      gappsWrapperArgs+=(--prefix XDG_DATA_DIRS : ${pkgs.adwaita-icon-theme}/share)
      gappsWrapperArgs+=(--prefix XDG_DATA_DIRS : ${pkgs.papirus-icon-theme}/share)
    '';

  };
  in {
    packages.${system}.default = epic-bar;
    devShells.${system}.default = pkgs.mkShell {
      buildInputs = with pkgs; [
        rustToolchain
        preview-grim
        epic-bar  
      ] ++ epic-bar.buildInputs;  

      nativeBuildInputs = epic-bar.nativeBuildInputs;
    };
  };
}
