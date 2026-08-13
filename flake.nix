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

          # Packages only need to produce the binary; the workspace-wide `test`
          # check below is the single owner of test execution. Running `cargo
          # test -p <crate>` in every package's checkPhase would rebuild the
          # shared dependency graph once per crate (cargo fingerprints do not
          # survive the differing sandbox source paths), which is what made the
          # old per-crate check derivations so slow.
          mkPkg =
            name:
            craneLib.buildPackage (
              individualCrateArgs name
              // lib.optionalAttrs (name == "sui") { inherit version; }
              // {
                doCheck = false;
              }
            );
        in
        {
          checks = {
            test = craneLib.cargoTest (commonArgs // { inherit cargoArtifacts; });
          };

          packages = builtins.listToAttrs (map (name: lib.nameValuePair name (mkPkg name)) crates) // {
            default = self'.packages.sui;
          };

          devShells.default = pkgs.mkShellNoCC {
            inputsFrom = [ config.pre-commit.devShell ];
            env = {
              SUI_LLM_BASE_URL = "http://localhost:11434/v1";
              SUI_LLM_API_KEY = "ollama";
              SUI_LLM_MODEL = "gemma4:e4b";
            };
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
