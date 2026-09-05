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
        Peasy's only durable state: a generated NixOS module imported by the
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
    ++ lib.optional (cfg.tray.enable && gnomeEnabled) pkgs.gnomeExtensions.appindicator
    ++ lib.optional (
      cfg.desktop.enable && config.services.desktopManager.plasma6.enable
    ) pkgs.kdePackages.kconfig;
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

    systemd.services.peasy-system = {
      description = "Peasy typed NixOS configuration service";
      wantedBy = [ "multi-user.target" ];
      after = [ "nix-daemon.socket" ];
      requires = [ "nix-daemon.socket" ];
      # Applying a reviewed generation must not terminate the IPC request that
      # initiated it. The daemon can pick up a changed unit on the next normal
      # restart or boot.
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
          ]
          ++ rebuildArguments
        );
        Restart = "on-failure";
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
