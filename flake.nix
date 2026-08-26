{
  description = "HAL-9001 — Central TUI de Controle do Sistema & Assistente de Sistema (Rust/ratatui)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages = rec {
          hal-9001 = pkgs.callPackage ./default.nix { };
          default = hal-9001;
        };

        apps = rec {
          hal9001 = flake-utils.lib.mkApp {
            drv = self.packages.${system}.default;
            name = "hal9001";
          };
          default = hal9001;
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            pkg-config
            dbus
            systemd
          ];
        };
      }
    );
}
