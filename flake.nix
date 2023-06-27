{
  inputs = {
    nixpkgs.url = "nixpkgs/release-23.05";
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
        pkgs = nixpkgs.legacyPackages."${system}";
        naersk-lib = naersk.lib."${system}";
        lib = pkgs.lib;
        naerskConfig = {
          pname = "dropstf";
          root = lib.sources.sourceByRegex (lib.cleanSource ./.) ["Cargo.*" "(src|benches|templates)(/.*)?" "sqlx-data.json"];

          SQLX_OFFLINE = true;
        };
      in rec {
        # `nix build`
        packages = rec {
          dropstf = naersk-lib.buildPackage naerskConfig;
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
          nativeBuildInputs = with pkgs; [rustc cargo bacon cargo-edit cargo-outdated clippy];
        };
      }
    )
    // {
      nixosModules.default = {
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

            enableUnixSocket = mkOption {
              type = types.bool;
              default = false;
              description = "listen to a unix socket instead of TCP";
            };

            tracingEndpoint = mkOption {
              type = types.nullOr types.str;
              default = null;
              description = "OTLP tracing endpoint";
            };
          };

          config = mkIf cfg.enable {
            systemd.services."dropstf" = let
              pkg = self.packages.${pkgs.system}.default;
              needIp = (cfg.enableUnixSocket == false) || (cfg.tracingEndpoint != null);
            in {
              wantedBy = ["multi-user.target"];
              script = "${pkg}/bin/dropstf";
              environment =
                (
                  if cfg.enableUnixSocket
                  then {
                    SOCKET = "/run/dropstf/drops.sock";
                  }
                  else {
                    PORT = toString cfg.port;
                  }
                )
                // (attrsets.optionalAttrs (cfg.tracingEndpoint != null) {
                  TRACING_ENDPOINT = cfg.tracingEndpoint;
                });

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
                RestrictAddressFamilies = ["AF_UNIX"] ++ (optionals needIp ["AF_INET" "AF_INET6"]);
                IPAddressDeny =
                  if needIp == false
                  then "any"
                  else "multicast";
                PrivateNetwork = needIp == false;
                RestrictRealtime = true;
                ProtectProc = "invisible";
                SystemCallFilter = ["@system-service" "~@resources" "~@privileged"];
                PrivateUsers = true;
                ProcSubset = "pid";
                RuntimeDirectory = "dropstf";
                RestrictSUIDSGID = true;
              };
            };
          };
        };
    };
}
