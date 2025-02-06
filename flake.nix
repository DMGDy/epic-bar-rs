{
  description = "Custom status bar for Hyprland compositor";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }: 
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

    buildInputs = with pkgs; [
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

  };
  in {
    packages.${system}.default = epic-bar;
    devShells.${system}.default = pkgs.mkShell {
      buildInputs = with pkgs; [
        rustToolchain
        epic-bar  
      ] ++ epic-bar.buildInputs;  

      nativeBuildInputs = epic-bar.nativeBuildInputs;
    };
  };
}
