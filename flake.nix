{
  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    naersk,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages."${system}";
        naersk-lib = naersk.lib."${system}";
      in rec {
        # `nix build`
        packages.dropstf = naersk-lib.buildPackage {
          pname = "dropstf";
          root = ./.;

          SQLX_OFFLINE = true;
        };
        defaultPackage = packages.dropstf;
        defaultApp = packages.dropstf;

        # `nix develop`
        devShell = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [rustc cargo bacon cargo-edit cargo-outdated];
        };
      }
    )
    // {
      nixosModule = {
        config,
        lib,
        pkgs,
        ...
      }:
        with lib; let
          cfg = config.services.dropstf;
        in {
          options.services.dropstf = {
            enable = mkEnableOption "drops.tf";

            user = mkOption {
              type = types.str;
              description = "user to run as";
            };

            databaseUrlFile = mkOption {
              type = types.str;
              description = "file containg DATABASE_URL variable";
            };

            streamApiKeyFile = mkOption {
              type = types.str;
              description = "file containing STEAM_API_KEY variable";
            };

            port = mkOption {
              type = types.int;
              default = 80;
              description = "port to listen on";
            };
          };

          config = mkIf cfg.enable {
            systemd.services."dropstf" = let
              pkg = self.defaultPackage.${pkgs.system};
            in {
              wantedBy = ["multi-user.target"];
              script = "${pkg}/bin/dropstf";
              environment = {
                PORT = toString cfg.port;
              };

              serviceConfig = {
                EnvironmentFile = [cfg.databaseUrlFile cfg.streamApiKeyFile];
                Restart = "on-failure";
                User = cfg.user;
                PrivateTmp = true;
                ProtectSystem = "strict";
                ProtectHome = true;
                NoNewPrivileges = true;
                PrivateDevices = true;
                ProtectClock = true;
                CapabilityBoundingSet = true;
                ProtectKernelLogs = true;
                ProtectControlGroups = true;
                SystemCallArchitectures = "native";
                ProtectKernelModules = true;
                RestrictNamespaces = true;
                MemoryDenyWriteExecute = true;
                ProtectHostname = true;
                LockPersonality = true;
                ProtectKernelTunables = true;
                RestrictAddressFamilies = "AF_INET AF_INET6 AF_UNIX";
                RestrictRealtime = true;
                ProtectProc = "noaccess";
                SystemCallFilter = ["@system-service" "~@resources" "~@privileged"];
                IPAddressDeny = "multicast";
                PrivateUsers = true;
                ProcSubset = "pid";
              };
            };
          };
        };
    };
}
