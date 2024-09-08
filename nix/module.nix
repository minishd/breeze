{ config
, pkgs
, lib
, ...
}:
with lib; let
  cfg = config.services.breeze;
  settingsFormat = pkgs.formats.toml { };
in
{
  options = {
    services.breeze = {
      enable = mkEnableOption "breeze file server";

      package = mkPackageOption self.packages.${system} "breeze";

      user = mkOption {
        type = types.str;
        default = "breeze";
        description = "User that `breeze` will run under";
      };

      group = mkOption {
        type = types.str;
        default = "breeze";
        description = "Group that `breeze` will run under";
      };

      extraGroups = mkOption {
        type = types.listOf types.str;
        default = [ ];
        description = "Supplementary groups";
      };

      settings = mkOption {
        type = settingsFormat.type;
        default = { };
        description = ''
          The *.toml configuration to run `breeze` with.
          There is no formal documentation, but there is an example in the [readme](https://git.min.rip/min/breeze/src/branch/main/README.md).
        '';
      };
    };
  };

  config = mkIf cfg.enable {
    users.users.${cfg.user} = {
      isSystemUser = true;
      inherit (cfg) group;
    };

    users.groups.${cfg.group} = { };

    systemd.services.breeze = {
      description = "breeze file server";
      after = [ "local-fs.target" "network.target" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = rec {
        User = cfg.user;
        Group = cfg.group;
        DynamicUser = false; # we write files, so don't do that
        SupplementaryGroups = cfg.extraGroups;
        StateDirectory = "breeze";
        CacheDirectory = "breeze";
        ExecStart = escapeShellArgs [
          "${cfg.package}/bin/breeze"
          "--config"
          (settingsFormat.generate "breeze.toml" cfg.settings)
        ];
        Restart = "on-failure";

        # Security Options #

        NoNewPrivileges = true; # implied by DynamicUser
        RemoveIPC = true; # implied by DynamicUser

        AmbientCapabilities = "";
        CapabilityBoundingSet = "";

        DeviceAllow = "";

        LockPersonality = true;

        PrivateTmp = true; # implied by DynamicUser
        PrivateDevices = true;
        PrivateUsers = true;

        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;

        RestrictNamespaces = true;
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
        RestrictRealtime = true;
        RestrictSUIDSGID = true; # implied by DynamicUser

        SystemCallArchitectures = "native";
        SystemCallErrorNumber = "EPERM";
        SystemCallFilter = [
          "@system-service"
          "~@keyring"
          "~@memlock"
          "~@privileged"
          "~@setuid"
        ];
      };
    };
  };
}
