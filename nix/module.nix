{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.peasy;
  package = cfg.package;
  gnomeEnabled = config.services.desktopManager.gnome.enable or false;
  appindicatorUuid = pkgs.gnomeExtensions.appindicator.extensionUuid;
  hostConfigurationDirectory = builtins.dirOf cfg.hostConfiguration;
  hostFlakeDirectory =
    if cfg.hostFlake == null then null else builtins.head (lib.splitString "#" cfg.hostFlake);
  hostSourceDirectory =
    if cfg.hostFlake == null then hostConfigurationDirectory else hostFlakeDirectory;
  managedModule =
    if cfg.managedModule == null then
      "${hostSourceDirectory}/.peasy/peasy-managed.nix"
    else
      cfg.managedModule;
  managedModuleDirectory = builtins.dirOf managedModule;
  rebuildArguments =
    if cfg.hostFlake == null then
      [
        "--host-configuration ${lib.escapeShellArg cfg.hostConfiguration}"
        "--managed-module ${lib.escapeShellArg managedModule}"
      ]
    else
      [
        "--host-flake ${lib.escapeShellArg cfg.hostFlake}"
        "--nixos-rebuild ${config.system.build.nixos-rebuild}/bin/nixos-rebuild"
        "--managed-module ${lib.escapeShellArg managedModule}"
      ];
  configuredDesktops =
    lib.optional gnomeEnabled "gnome"
    ++ lib.optional (config.services.desktopManager.plasma6.enable or false) "kde_plasma"
    ++ lib.optional (config.programs.hyprland.enable or false) "hyprland"
    ++ lib.optional (config.services.xserver.desktopManager.xfce.enable or false) "xfce"
    ++ lib.optional (config.services.xserver.desktopManager.lxqt.enable or false) "lxqt";
  installedSystemPackages = lib.sort builtins.lessThan (
    lib.unique (map lib.getName config.environment.systemPackages)
  );
  daemonIdentity = pkgs.writeText "peasy-daemon-identity.json" (
    builtins.toJSON {
      protocol = 2;
      executable = "${package}/libexec/peasy-system";
      nixpkgs = toString pkgs.path;
      inherit (cfg)
        hostConfiguration
        hostFlake
        configurationReadPaths
        protectHome
        ;
      inherit managedModule;
      resourceLimits = cfg.resourceLimits;
    }
  );
  exportConfiguration =
    if cfg.hostFlake == null then cfg.hostConfiguration else "${hostFlakeDirectory}/flake.nix";
in
{
  options.services.peasy = {
    enable = lib.mkEnableOption "Peasy natural-language NixOS and desktop assistant";

    desktop.enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Install Peasy's GTK application and graphical-session integration. Disable
        this on minimal or server systems to select the lean peasy-core package
        and omit all desktop/session service defaults.
      '';
    };

    package = lib.mkOption {
      type = lib.types.package;
      default =
        if cfg.desktop.enable then
          pkgs.callPackage ./package.nix { }
        else
          pkgs.callPackage ./package-core.nix { };
      defaultText = lib.literalExpression ''
        if config.services.peasy.desktop.enable then
          pkgs.callPackage <peasy/nix/package.nix> { }
        else
          pkgs.callPackage <peasy/nix/package-core.nix> { }
      '';
      description = "Peasy package to install.";
    };

    tray.enable = lib.mkOption {
      type = lib.types.bool;
      default = cfg.desktop.enable;
      defaultText = lib.literalExpression "config.services.peasy.desktop.enable";
      description = "Enable Peasy's generic StatusNotifier tray in compatible graphical sessions.";
    };

    hyprland.enable = lib.mkOption {
      type = lib.types.bool;
      default = cfg.desktop.enable;
      defaultText = lib.literalExpression "config.services.peasy.desktop.enable";
      description = ''
        Enable Peasy's Hyprland session integration defaults. The generic
        tray.enable option controls tray startup; a compatible bar such as
        Waybar must provide a tray host. Typed live control uses the
        hyprctl belonging to the running Hyprland session.
      '';
    };

    hyprland.authenticationAgent.enable = lib.mkOption {
      type = lib.types.bool;
      default = cfg.hyprland.enable && (config.programs.hyprland.enable or false);
      description = ''
        Start a graphical Polkit authentication agent in Hyprland sessions.
        Disable this if your session already starts an authentication agent.
      '';
    };

    ollama.enable = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Enable NixOS's local Ollama service for Peasy. After rebuilding, pull
        at least one model with `ollama pull MODEL`, then select it from the
        Peasy settings cog. The service remains bound to its local default.
      '';
    };

    hostConfiguration = lib.mkOption {
      type = lib.types.str;
      default = "/etc/nixos/configuration.nix";
      description = ''
        Absolute path to the trusted NixOS module that Peasy evaluates together
        with the Peasy-owned managed module.
      '';
    };

    appImages.trustedHashes = lib.mkOption {
      type = lib.types.nullOr (lib.types.attrsOf (lib.types.listOf lib.types.str));
      default = null;
      example = {
        "owner/project" = [ "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=" ];
      };
      description = ''
        Administrator-approved SHA-256 SRI hashes for external AppImages, keyed
        by lowercase GitHub owner/repository. The default, null, permits external
        installs after source review and administrator authentication, without
        preapproved hashes. Set an attribute set to enforce an exact hash
        allowlist, or an empty set to disable new external installs. Verify
        allowlisted hashes independently against a trusted publisher. Existing
        installations can still be rebuilt and removed.
      '';
    };

    resourceLimits.memoryMax = lib.mkOption {
      type = lib.types.str;
      default = "6G";
      description = "Memory limit for Peasy and its Nix evaluation children; increase for unusually large host configurations.";
    };

    managedModule = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/etc/nixos/.peasy/peasy-managed.nix";
      description = ''
        Peasy's desired state: a generated NixOS module imported by the
        host configuration. The default places it in a Peasy-owned directory
        beside the host configuration or flake.
      '';
    };

    hostFlake = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/etc/nixos#my-host";
      description = ''
        Optional trusted host flake reference. When null, the default, Peasy
        evaluates hostConfiguration directly and requires no host flake. Set
        this only when the host's complete module graph exists in flake.nix.
      '';
    };

    configurationReadPaths = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "/home/alice/src/peasy" ];
      description = ''
        Additional trusted paths made read-only inside the daemon sandbox while
        evaluating the host configuration. This is normally empty. It is useful
        only when configuration.nix imports a local module below a protected home
        directory; prefer store-backed or /etc/nixos modules for deployments.
      '';
    };

    protectHome = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Hide /home and /root from peasy-system. Disable only when the trusted
        host configuration depends on paths below a home directory that cannot
        instead be listed narrowly in configurationReadPaths.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    # Garcon follows application-directory inodes into the immutable store;
    # replacing /run/current-system therefore does not update XFCE's menus.
    # Observe that switch in the existing menu process, with no polling or
    # per-user launcher copies. Only XFCE installations need this override.
    nixpkgs.overlays =
      lib.mkIf (cfg.desktop.enable && config.services.xserver.desktopManager.xfce.enable)
        [
          (_final: prev: {
            garcon = prev.garcon.overrideAttrs (old: {
              patches = (old.patches or [ ]) ++ [ ./patches/garcon-nixos-generation.patch ];
            });
          })
        ];
    assertions = [
      {
        assertion = lib.hasPrefix "/" cfg.hostConfiguration;
        message = "services.peasy.hostConfiguration must be an absolute path";
      }
      {
        assertion = cfg.hostFlake == null || lib.hasPrefix "/" hostFlakeDirectory;
        message = "services.peasy.hostFlake must use an absolute local path";
      }
      {
        assertion = lib.hasPrefix "${hostSourceDirectory}/.peasy/" managedModule;
        message = "services.peasy.managedModule must be inside the host's .peasy directory";
      }
      {
        assertion = lib.all (lib.hasPrefix "/") cfg.configurationReadPaths;
        message = "services.peasy.configurationReadPaths entries must be absolute paths";
      }
      {
        assertion = !cfg.tray.enable || cfg.desktop.enable;
        message = "services.peasy.tray.enable requires services.peasy.desktop.enable";
      }
      {
        assertion = !cfg.hyprland.enable || cfg.desktop.enable;
        message = "services.peasy.hyprland.enable requires services.peasy.desktop.enable";
      }
    ];

    security.polkit.enable = true;
    # The system exporter reads this stable path. Do not rely on a desktop
    # module linking all of /share (XFCE links only selected subdirectories).
    environment.pathsToLink = lib.optional cfg.desktop.enable "/share/peasy";
    environment.systemPackages = [
      package
      (pkgs.writeTextDir "share/polkit-1/actions/io.github.peasy.policy" ''
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE policyconfig PUBLIC "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN" "http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
        <policyconfig>
          <vendor>Peasy</vendor>
          <action id="io.github.peasy.apply">
            <description>Apply a Peasy system change</description>
            <message>Authentication is required to apply this Peasy system change</message>
            <defaults>
              <allow_any>auth_admin</allow_any>
              <allow_inactive>auth_admin</allow_inactive>
              <allow_active>auth_admin</allow_active>
            </defaults>
          </action>
        </policyconfig>
      '')
    ]
    # AppIndicator launches gjs by name to rediscover existing tray items.
    ++ lib.optionals (cfg.tray.enable && gnomeEnabled) [
      pkgs.gnomeExtensions.appindicator
      pkgs.gjs
    ]
    ++ lib.optional (
      cfg.desktop.enable && config.services.desktopManager.plasma6.enable
    ) pkgs.kdePackages.kconfig;
    environment.etc."peasy/daemon-identity.json".source = daemonIdentity;
    environment.etc."peasy/appimage-policy.json".text = builtins.toJSON cfg.appImages.trustedHashes;

    systemd.user.services.peasy-polkit-agent = lib.mkIf cfg.hyprland.authenticationAgent.enable {
      description = "Polkit authentication for Peasy in Hyprland";
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session-pre.target" ];
      unitConfig.ConditionEnvironment = "HYPRLAND_INSTANCE_SIGNATURE";
      serviceConfig = {
        Type = "simple";
        ExecStart = "${pkgs.polkit_gnome}/libexec/polkit-gnome-authentication-agent-1";
      };
    };

    systemd.tmpfiles.rules = [
      "d ${managedModuleDirectory} 0755 root root -"
    ];

    environment.etc."peasy/system-profile.json" = {
      mode = "0444";
      text = builtins.toJSON {
        nixos_version = config.system.nixos.release;
        nix_system = pkgs.stdenv.hostPlatform.system;
        configured_desktops = configuredDesktops;
        peasy_variant = if cfg.desktop.enable then "desktop" else "headless";
        installed_system_packages = installedSystemPackages;
      };
    };

    environment.etc."peasy/host-configuration-path" = {
      mode = "0444";
      text = "${exportConfiguration}\n";
    };

    # Preserve the module path as it appears in the administrator's source so
    # the system exporter can replace a checkout/store-specific reference with
    # the Peasy source included in the portable bundle.
    environment.etc."peasy/module-import-path" = {
      mode = "0444";
      text = "${builtins.unsafeDiscardStringContext (toString ./module.nix)}\n";
    };

    # A Peasy-generated generation embeds its reviewed state in /etc. When an
    # older generation is activated, make that explicit rollback durable by
    # restoring the Peasy-owned source module to the selected generation's
    # state. Normal forward switches simply rewrite the same canonical state.
    system.activationScripts.peasy-managed-state = lib.stringAfter [ "etc" ] ''
      if [ -e /etc/peasy/state.json ]; then
        ${package}/libexec/peasy-system \
          --reconcile-managed-state /etc/peasy/state.json \
          --managed-module ${lib.escapeShellArg managedModule}
      fi
    '';

    networking.networkmanager.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    hardware.bluetooth.enable = lib.mkIf cfg.desktop.enable (lib.mkDefault true);
    services.ollama.enable = lib.mkIf cfg.ollama.enable true;

    services.desktopManager.gnome = lib.mkIf (cfg.tray.enable && gnomeEnabled) {
      extraGSettingsOverridePackages = [ pkgs.gnome-shell ];
      extraGSettingsOverrides = ''
        [org.gnome.shell]
        enabled-extensions=['${appindicatorUuid}']
      '';
    };

    environment.etc."xdg/autostart/peasy-panel.desktop" = lib.mkIf (cfg.tray.enable && gnomeEnabled) {
      mode = "0444";
      text = ''
        [Desktop Entry]
        Type=Application
        Name=Enable StatusNotifier support
        Comment=Enable GNOME compatibility for the generic Peasy tray
        Exec=${pkgs.writeShellScript "peasy-gnome-tray-compatibility" ''
          ${pkgs.gnome-shell}/bin/gnome-extensions disable peasy@peasy-nixos.github.io || true
          exec ${pkgs.gnome-shell}/bin/gnome-extensions enable ${appindicatorUuid}
        ''}
        Terminal=false
        OnlyShowIn=GNOME;
        X-GNOME-Autostart-enabled=true
        NoDisplay=true
      '';
    };

    environment.etc."xdg/autostart/peasy-tray.desktop" = lib.mkIf cfg.tray.enable {
      mode = "0444";
      text = ''
        [Desktop Entry]
        Type=Application
        Name=Peasy
        Comment=Open Peasy from your desktop tray
        Exec=${package}/bin/peasy-tray --ui ${package}/bin/peasy-ui
        Terminal=false
        NoDisplay=true
      '';
    };

    # Apply the generation's validated appearance values in each active user
    # session. The service is unprivileged and the CLI accepts only the closed
    # ThemeSettings JSON written into /etc by Peasy's generated module.
    systemd.user.services.peasy-theme-sync = lib.mkIf cfg.desktop.enable {
      description = "Synchronize Peasy appearance for the current desktop";
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session-pre.target" ];
      unitConfig.ConditionPathExists = "/etc/peasy/theme.json";
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${package}/bin/peasy --sync-theme";
        NoNewPrivileges = true;
        PrivateTmp = true;
      };
    };

    # Keep live appearance aligned when switching or rolling back between
    # Peasy-generated NixOS generations without requiring another login.
    systemd.user.paths.peasy-theme-sync = lib.mkIf cfg.desktop.enable {
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      pathConfig.PathChanged = "/etc/peasy/theme.json";
    };

    # Nixpkgs' glib-appinfo-watch.patch monitors /nix/var/nix/profiles.
    # nix-env updates that profile BEFORE activation updates /run/current-system
    # (and hence PATH). If the desktop reloads in between, new Exec/TryExec
    # commands are not runnable yet and its cached app list omits them.
    # Notify the existing monitor again once the running generation changes.
    # Touch only the profile SYMLINK, never its store target. No launcher copies,
    # extra desktop IDs, per-user state, or desktop restarts are needed.
    systemd.services.peasy-applications-refresh = lib.mkIf cfg.desktop.enable {
      description = "Notify desktop application monitors after NixOS activation";
      # Also notify on the upgrade that first introduces the path unit: its
      # watcher starts after that activation has already changed the symlink.
      wantedBy = [ "multi-user.target" ];
      unitConfig.ConditionPathIsSymbolicLink = "/nix/var/nix/profiles/system";
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${pkgs.coreutils}/bin/touch --no-create --no-dereference /nix/var/nix/profiles/system";
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ "/nix/var/nix/profiles" ];
        NoNewPrivileges = true;
        PrivateTmp = true;
        CapabilityBoundingSet = "";
      };
    };

    systemd.paths.peasy-applications-refresh = lib.mkIf cfg.desktop.enable {
      wantedBy = [ "multi-user.target" ];
      pathConfig = {
        # Watch the containing directory: watching the symlink itself follows
        # its old target and misses atomic replacements. Other top-level /run
        # changes may also send a harmless refresh; there is no saved state.
        PathChanged = "/run";
        Unit = "peasy-applications-refresh.service";
      };
    };

    systemd.services.peasy-system = {
      description = "Peasy typed NixOS configuration service";
      wantedBy = [ "multi-user.target" ];
      after = [ "nix-daemon.socket" ];
      requires = [ "nix-daemon.socket" ];
      # Applying a reviewed generation must not terminate the IPC request that
      # initiated it. The daemon drains active requests and exits when the
      # fully switched generation advertises a new daemon identity.
      restartIfChanged = false;
      stopIfChanged = false;
      serviceConfig = {
        Type = "simple";
        Group = "wheel";
        ExecStart = lib.concatStringsSep " " (
          [
            "${package}/libexec/peasy-system"
            "--nix ${pkgs.nix}/bin/nix"
            "--systemctl ${pkgs.systemd}/bin/systemctl"
            "--pkcheck ${pkgs.polkit}/bin/pkcheck"
            "--nixpkgs ${pkgs.path}"
            "--system ${pkgs.stdenv.hostPlatform.system}"
            "--identity ${daemonIdentity}"
          ]
          ++ rebuildArguments
        );
        # The daemon drains requests and exits after detecting a newer active
        # identity. Restart uses the newly loaded unit, including its Nixpkgs.
        Restart = "always";
        RestartSec = 2;

        RuntimeDirectory = "peasy";
        RuntimeDirectoryMode = "0755";
        UMask = "0077";

        CapabilityBoundingSet = "";
        AmbientCapabilities = "";
        SystemCallArchitectures = "native";
        SystemCallFilter = [
          "@system-service"
          "~@mount"
          "~@debug"
          "~@reboot"
          "~@swap"
        ];
        SystemCallErrorNumber = "EPERM";
        # Nix's garbage collector reads /proc/stat. Hiding non-process proc
        # entries breaks that runtime assumption; do not use ProcSubset=pid.
        # /proc/<peer>/stat is needed to bind Polkit to the client's start time.
        # Empty capabilities prevent ptrace and proc-root sandbox escapes.
        ProtectHostname = true;
        MemoryDenyWriteExecute = true;
        LimitCORE = 0;
        TasksMax = 256;
        # Cold Nixpkgs evaluation can exceed 4 GiB. A lower soft threshold
        # causes reclaim thrashing; keep the configurable hard ceiling only.
        MemoryMax = cfg.resourceLimits.memoryMax;
        MemorySwapMax = "1G";
        CPUWeight = 20;

        ProtectHome = if cfg.protectHome then "tmpfs" else "read-only";
        ProtectSystem = "strict";
        ReadOnlyPaths = [ hostSourceDirectory ];
        BindReadOnlyPaths = cfg.configurationReadPaths;
        ReadWritePaths = [
          "/run/peasy"
          managedModuleDirectory
        ];
        PrivateTmp = true;
        PrivateDevices = true;
        NoNewPrivileges = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        RestrictRealtime = true;
        RestrictNamespaces = true;
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_NETLINK"
        ];
        IPAddressDeny = "any";
      };
    };

    systemd.services.peasy-activate = {
      description = "Activate a Peasy-validated NixOS generation";
      # switch-to-configuration reconciles systemd units while this oneshot is
      # still running.  Never let that reconciliation terminate the helper
      # which is performing the switch; the updated unit is used on its next
      # invocation.
      restartIfChanged = false;
      stopIfChanged = false;
      serviceConfig = {
        Type = "oneshot";
        TimeoutStartSec = "15min";
        ExecStart = "${package}/libexec/peasy-system --activate --runtime-dir /run/peasy --nix-env ${pkgs.nix}/bin/nix-env";
        UMask = "0077";

        # NixOS activation legitimately updates the system profile, /etc,
        # users, boot state, kernel settings, devices, and user units. A
        # filesystem or device sandbox makes switch-to-configuration fail.
        # This narrowly-scoped helper accepts no user input: it consumes only
        # the root-owned, validated store path written by peasy-system.
      };
    };
  };
}
