{
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

    package = mkOption {
      type = types.package;
      defaultText = literalExpression "pkgs.shelve";
      description = "package to use";
    };
  };

  config = mkIf cfg.enable {
    systemd.services."dropstf" = let
      needIp = (cfg.enableUnixSocket == false) || (cfg.tracingEndpoint != null);
    in {
      wantedBy = ["multi-user.target"];
      script = "${cfg.package}/bin/dropstf";
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
}
