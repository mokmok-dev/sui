{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    git-hooks = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    inputs:
    let
      version =
        let
          d = inputs.self.lastModifiedDate;
          dezero =
            s:
            let
              m = builtins.match "0(.*)" s;
            in
            if m == null then s else builtins.head m;
          date = "${builtins.substring 0 4 d}.${dezero (builtins.substring 4 2 d)}.${
            dezero (builtins.substring 6 2 d)
          }";
          rev = inputs.self.shortRev or inputs.self.dirtyShortRev or "dirty";
        in
        "${date}-${rev}";
    in
    inputs.flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];

      imports = [
        inputs.treefmt-nix.flakeModule
        inputs.git-hooks.flakeModule
      ];

      perSystem =
        {
          self',
          config,
          lib,
          system,
          ...
        }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ inputs.rust-overlay.overlays.default ];
          };
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (inputs.crane.mkLib pkgs).overrideToolchain rustToolchain;
          src = craneLib.cleanCargoSource ./.;
          commonArgs = {
            inherit src;
            pname = "sui-workspace";
            strictDeps = true;
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          crates = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.members;

          individualCrateArgs =
            name:
            commonArgs
            // {
              pname = name;
              cargoExtraArgs = "-p ${name}";
              inherit cargoArtifacts;
            };

          mkTest = name: lib.nameValuePair "${name}-test" (craneLib.cargoTest (individualCrateArgs name));

          mkPkg =
            name:
            craneLib.buildPackage (
              individualCrateArgs name // lib.optionalAttrs (name == "sui") { inherit version; }
            );
        in
        {
          checks = builtins.listToAttrs (map mkTest crates);

          packages = builtins.listToAttrs (map (name: lib.nameValuePair name (mkPkg name)) crates) // {
            default = self'.packages.sui;
          };

          devShells.default = pkgs.mkShellNoCC {
            inputsFrom = [ config.pre-commit.devShell ];
            packages = [ rustToolchain ];
          };

          pre-commit.settings = {
            hooks = {
              actionlint.enable = true;
              deadnix.enable = true;
              statix = {
                enable = true;
                settings.ignore = [ ".direnv/**" ];
              };
            };
          };

          treefmt = {
            projectRootFile = "flake.nix";
            programs = {
              nixfmt.enable = true;
              rustfmt.enable = true;
              rustfmt.package = rustToolchain;
              taplo.enable = true;
            };
          };
        };
    };
}
