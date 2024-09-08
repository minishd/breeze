{
  description = "breeze file server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self
    , nixpkgs
    , crane
    , flake-utils
    , ...
    }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = nixpkgs.legacyPackages.${system};

      craneLib = crane.mkLib pkgs;

      # Common arguments can be set here to avoid repeating them later
      # Note: changes here will rebuild all dependency crates
      commonArgs = {
        src = craneLib.cleanCargoSource ./.;
        strictDeps = true;

        buildInputs =
          [
            pkgs.openssl
          ]
          ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            # Additional darwin specific inputs can be set here
            pkgs.libiconv
          ];
      };

      breeze = craneLib.buildPackage (commonArgs
        // {
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Additional environment variables or build phases/hooks can be set
        # here *without* rebuilding all dependency crates
        # MY_CUSTOM_VAR = "some value";
      });
    in
    {
      checks = {
        inherit breeze;
      };

      packages.default = breeze;

      apps.default = flake-utils.lib.mkApp {
        drv = breeze;
      };

      devShells.default = craneLib.devShell {
        # Inherit inputs from checks.
        checks = self.checks.${system};

        # Additional dev-shell environment variables can be set directly
        # MY_CUSTOM_DEVELOPMENT_VAR = "something else";

        # Extra inputs can be added here; cargo and rustc are provided by default.
        packages = with pkgs; [
          alejandra
          rewrk
        ];
      };

      nixosModules.breeze =
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

                package = mkOption {
                  type = types.package;
                  default = breeze;
                  description = "Package for `breeze` to use";
                };

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
          };
    });
}
