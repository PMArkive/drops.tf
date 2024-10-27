{
  inputs = {
    nixpkgs.url = "nixpkgs/release-24.05";
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    naersk,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [
          (import ./overlay.nix)
        ];
        pkgs = (import nixpkgs) {
          inherit system overlays;
        };
        inherit (pkgs) lib callPackage rust-bin mkShell;

        naersk-lib = callPackage naersk {};
        naerskConfig = {
          inherit (pkgs.dropstf) src pname;

          SQLX_OFFLINE = true;
        };
      in rec {
        # `nix build`
        packages = rec {
          dropstf = pkgs.dropstf;
          check = naersk-lib.buildPackage (naerskConfig
            // {
              mode = "check";
            });
          clippy = naersk-lib.buildPackage (naerskConfig
            // {
              mode = "clippy";
            });
          dockerImage = pkgs.dockerTools.buildImage {
            name = "icewind1991/drops.tf";
            tag = "latest";
            copyToRoot = [dropstf];
            config = {
              Cmd = ["${dropstf}/bin/dropstf"];
            };
          };
          default = dropstf;
        };

        # `nix develop`
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [rustc cargo bacon cargo-edit cargo-outdated clippy sqlx-cli];
        };
      }
    )
    // {
      overlays.default = import ./overlay.nix;
      nixosModules.default = {
        pkgs,
        config,
        lib,
        ...
      }: {
        imports = [./module.nix];
        config = lib.mkIf config.services.dropstf.enable {
          nixpkgs.overlays = [self.overlays.default];
          services.dropstf.package = lib.mkDefault pkgs.dropstf;
        };
      };
    };
}
