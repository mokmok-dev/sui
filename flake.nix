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

          # Build the whole workspace in a single derivation (no `-p <crate>`)
          # so the final `cargo build --locked` resolves exactly the same
          # feature sets as the `buildDepsOnly` artifact. Building per-crate
          # (`cargo build -p sui`) resolves a *subset* of the workspace
          # features — e.g. `sui-workflow` pulls `rhai`, which enables
          # `once_cell`'s `portable-atomic` feature — so cargo recompiles
          # those crates on every build: the shared dependency artifact never
          # matches the final build and the cache stops working.
          #
          # The output is only the `sui` binary (every other member is
          # lib-only); do not reintroduce `-p`, `--features`, or asymmetric
          # `cargoExtraArgs` here, or the feature-resolution mismatch returns.
          packageArgs = commonArgs // {
            pname = "sui";
            inherit version;
            # The workspace-wide `test` check below is the single owner of
            # test execution; running tests here as well would only recompile
            # and re-run them.
            doCheck = false;
          };
        in
        {
          checks = {
            test = craneLib.cargoTest (
              commonArgs
              // {
                inherit cargoArtifacts;
                nativeBuildInputs = [ pkgs.gitMinimal ];
              }
            );
          };

          packages = {
            sui = craneLib.buildPackage (packageArgs // { inherit cargoArtifacts; });
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
