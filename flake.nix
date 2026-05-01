{
  description = "lojix-cli-v2 — forked development repo for the next generation of the CriomOS deploy CLI.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";

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
              "git+https://github.com/LiGoldragon/horizon-rs#82843677ad5583515afd91a52febd0e08304ec2d" =
                "sha256-HzyN4m51s9OcsGNbu/Gt2pYlyjkzBqdHColaajQY9nY=";
              "git+https://github.com/LiGoldragon/nota-codec.git#f9112ee308824014501742cf0670ccd1e0d56aff" =
                "sha256-NSry8yxZkfN9n6kYdi3VnU8AAPsZtmxnnN8o/sLWksM=";
              "git+https://github.com/LiGoldragon/nota-derive.git?branch=main#d936e20bd4bb6b09999f5efac5f537f368598ed1" =
                "sha256-/sM4CHMnoXg6QVZPeH/9E/h1wmfGBOliWEwoW9Rq0ik=";
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
            pname = "lojix-cli-v2";
            meta.mainProgram = "lojix-cli-v2";
            # Skip the test phase: tests/eval.rs uses
            # env!("CARGO_BIN_EXE_lojix-cli-v2") which is only set at
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
