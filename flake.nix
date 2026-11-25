{
  description = "Craft shared libraries";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    {
      self,
      nixpkgs,
    }:
    let
      inherit (nixpkgs) lib;

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];

      forAllSystems = f: lib.genAttrs systems (system: f (import nixpkgs { inherit system; }) system);

      runtimeLibs =
        pkgs:
        with pkgs;
        lib.optionals stdenv.hostPlatform.isLinux [
          wayland
          libxkbcommon
          libx11
          libxcb
          libxcursor
          libxi
          libxrandr
          libxinerama
          vulkan-loader
          libglvnd
          dbus
          alsa-lib
          fontconfig
          freetype
        ];
    in
    {
      devShells = forAllSystems (
        pkgs: _:
        {
          default = pkgs.mkShell {
            packages =
              with pkgs;
              [
                cargo
                rustc
                clippy
                rustfmt
                rust-analyzer
                nixd
                nixfmt
                pkg-config
                git
              ]
              ++ runtimeLibs pkgs;

            LD_LIBRARY_PATH = lib.makeLibraryPath (runtimeLibs pkgs);
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            RUST_BACKTRACE = "1";
          };
        }
      );

      formatter = forAllSystems (pkgs: _: pkgs.nixfmt);
    };
}
