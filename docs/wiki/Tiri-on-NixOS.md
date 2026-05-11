# Tiri on NixOS

Tiri is this fork of niri. The binary and Nix package are still named `niri`, so the simplest NixOS setup is to override `pkgs.niri` and let existing `programs.niri` or session configuration keep using the normal package name.

## Flake Input

Add the fork as an input in `/etc/nixos/flake.nix`:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    tiri = {
      url = "github:JettChenT/tiri";
      flake = false;
    };
  };

  outputs =
    inputs@{ self, nixpkgs, ... }:
    {
      nixosConfigurations.nixos = nixpkgs.lib.nixosSystem {
        system = "aarch64-linux";
        specialArgs = { inherit inputs; };
        modules = [
          ./configuration.nix
        ];
      };
    };
}
```

For local development, use a path input instead:

```nix
tiri = {
  url = "path:/home/jettc/osdev/niri";
  flake = false;
};
```

## Overlay

Create `/etc/nixos/tiri-overlay.nix`:

```nix
{ inputs, ... }:

{
  nixpkgs.overlays = [
    (_final: prev: {
      niri = prev.niri.overrideAttrs (old: {
        version = "${old.version}-tiri";
        src = inputs.tiri;
        cargoHash = "sha256-gfnalA3qI3a9h3PvsxgQLCrzapfjLLkxhTMJpwRh+ro=";
        doCheck = false;
        doInstallCheck = false;
        env = (old.env or { }) // {
          NIRI_BUILD_COMMIT = "tiri";
        };
      });
    })
  ];
}
```

Import the overlay from `/etc/nixos/configuration.nix`:

```nix
{
  imports = [
    ./hardware-configuration.nix
    ./tiri-overlay.nix
  ];

  programs.niri.enable = true;
}
```

After changing the input from raw niri to tiri, rebuild:

```sh
sudo nixos-rebuild switch --flake /etc/nixos#nixos
```

If Cargo dependencies change in this fork, Nix may report a hash mismatch for `cargoHash`. Replace the value in `tiri-overlay.nix` with the hash printed by the failed build, then run the rebuild again.

## Screenshot Notification Option

This fork adds `notify` to `screenshot-window`. It defaults to `true`, matching upstream behavior. Set it to `false` to capture a window without showing the desktop screenshot notification:

```kdl
binds {
    Alt+Print { screenshot-window notify=false; }
}
```
