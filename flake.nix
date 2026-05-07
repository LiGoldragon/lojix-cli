{
  description = "CriomOS deploy CLI.";

  inputs = {
    nixpkgs.url = "github:LiGoldragon/nixpkgs?ref=main";

    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";

    crane.url = "github:ipetkov/crane";
  };

  outputs =
    { self, nixpkgs, fenix, crane }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forSystems = f: nixpkgs.lib.genAttrs systems (s: f s);

      mkContext = system:
        let
          pkgs = import nixpkgs { inherit system; };
          toolchain = fenix.packages.${system}.fromToolchainFile {
            file = ./rust-toolchain.toml;
            sha256 = "sha256-gh/xTkxKHL4eiRXzWv8KP7vfjSk61Iq48x47BEDFgfk=";
          };
          craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
          src = craneLib.cleanCargoSource ./.;
          # Git-URL deps (horizon-lib, nota-codec, nota-derive) — crane
          # vendors them via builtins.fetchGit; output hashes pin them.
          cargoVendorDir = craneLib.vendorCargoDeps {
            inherit src;
            outputHashes = {
              "git+https://github.com/LiGoldragon/horizon-rs#13c40ace5d435cfbf36532ca3c8659f11acb6461" =
                "sha256-z1EEfJNqKmkD0C1aEwIJdZNBDyvqRwEqUvfKYuBWtrM=";
              "git+https://github.com/LiGoldragon/nota-codec.git#85e21b4487bbd602f65f1b559029a24d9f5689f3" =
                "sha256-5eVNhjCCYqT4FdlardQhkKaeroIOYPztvNXEKlr/4r4=";
              "git+https://github.com/LiGoldragon/nota-derive.git?branch=main#8684dacf9346c5523ab51d54fe742fe2608461f0" =
                "sha256-z+sBGTUrPdkV64apZIoAquzudCzhw0lhmwCfwFPE0u0=";
            };
          };
          commonArgs = {
            inherit src cargoVendorDir;
            strictDeps = true;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.openssl ];
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        { inherit pkgs toolchain craneLib commonArgs cargoArtifacts; };
    in
    {
      packages = forSystems (system:
        let ctx = mkContext system; in
        {
          default = ctx.craneLib.buildPackage (ctx.commonArgs // {
            inherit (ctx) cargoArtifacts;
            # pname must match Cargo.toml's [[bin]] name so `nix run`
            # finds bin/<pname>.
            pname = "lojix-cli";
            meta.mainProgram = "lojix-cli";
            # Skip the test phase: tests/eval.rs uses
            # env!("CARGO_BIN_EXE_lojix-cli") which is only set at
            # cargo-test-runtime, not compile-time. Tests still run via
            # `nix build .#checks.<system>.default` which uses
            # craneLib.cargoTest (different test invocation).
            doCheck = false;
          });
        });

      checks = forSystems (system:
        let ctx = mkContext system; in
        {
          default = ctx.craneLib.cargoTest (ctx.commonArgs // {
            inherit (ctx) cargoArtifacts;
          });
        });

      devShells = forSystems (system:
        let ctx = mkContext system; in
        {
          default = ctx.pkgs.mkShell {
            packages = [
              ctx.toolchain
              ctx.pkgs.pkg-config
              ctx.pkgs.openssl
              ctx.pkgs.nix
            ];
          };
        });
    };
}
