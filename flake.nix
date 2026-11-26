{
  description = "Rexy native Linux client for Rex";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      crane,
      nixpkgs,
      rust-overlay,
    }:
    let
      lock = builtins.fromTOML (builtins.readFile ./Cargo.lock);
      bindings = nixpkgs.lib.findFirst (pkg: pkg.name == "libghostty-vt-sys")
        (throw "Cargo.lock is missing libghostty-vt-sys") lock.package;
      gitSource = builtins.match "git\\+([^?#]+)([?][^#]*)?#([0-9a-f]{40})" bindings.source;
      bindingsSrc = builtins.fetchGit {
        url = builtins.elemAt gitSource 0;
        rev = builtins.elemAt gitSource 2;
      };
      buildScript = builtins.readFile (bindingsSrc + "/crates/libghostty-vt-sys/build.rs");
      commit = builtins.match ''.*const GHOSTTY_COMMIT: &str = "([0-9a-f]{40})";.*'' buildScript;
      ghostty = (builtins.fetchTree {
        type = "github";
        owner = "ghostty-org";
        repo = "ghostty";
        rev = if commit == null then throw "Cannot find GHOSTTY_COMMIT in libghostty-vt-sys/build.rs"
              else builtins.head commit;
      }).outPath;
      forLinux = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
      ];
    in
    {
      packages = forLinux (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          rust = (rust-overlay.lib.mkRustBin { } pkgs).fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (crane.mkLib pkgs).overrideToolchain rust;
          runtimeLibraries = with pkgs; [
            fontconfig
            libGL
            libxkbcommon
            wayland
            libx11
            libxcursor
            libxi
            libxrandr
            libxcb
          ];
          commonArgs = {
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                (craneLib.fileset.commonCargoSources ./.)
                ./assets/fonts
                ./assets/icons
                ./assets/dev.rexy.client.svg
                ./assets/dev.rexy.client.desktop
              ];
            };
            nativeBuildInputs = with pkgs; [
              pkg-config
              makeWrapper
            ];
            buildInputs = runtimeLibraries;
            GHOSTTY_SOURCE_DIR = ghostty;
            GHOSTTY_ZIG_SYSTEM_DIR = pkgs.callPackage (ghostty + "/deps/packages.nix") { };
            ZIG = "${pkgs.zig_0_17}/bin/zig";
          };
        in
        rec {
          default = rexy;
          rexy = craneLib.buildPackage (
            commonArgs
            // {
              cargoArtifacts = craneLib.buildDepsOnly commonArgs;
              postInstall = ''
                install -Dm644 assets/dev.rexy.client.desktop \
                  "$out/share/applications/dev.rexy.client.desktop"
                install -Dm644 assets/dev.rexy.client.svg \
                  "$out/share/icons/hicolor/scalable/apps/dev.rexy.client.svg"
              '';
              postFixup = ''
                wrapProgram "$out/bin/rexy" \
                  --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath runtimeLibraries}" \
                  --suffix PATH : "${pkgs.lib.makeBinPath [ pkgs.openssh ]}"
              '';
              meta.mainProgram = "rexy";
            }
          );
        }
      );
      checks = forLinux (system: {
        inherit (self.packages.${system}) rexy;
      });
    };
}
