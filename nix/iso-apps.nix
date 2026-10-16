{ lib, pkgs, ... }:
{
  # Shared by the live image, installed target and offline installation cache.
  programs.appimage = {
    enable = lib.mkDefault true;
    binfmt = lib.mkDefault true;
  };
  environment.systemPackages = [ (pkgs.callPackage ./anagram.nix { }) ];
}
