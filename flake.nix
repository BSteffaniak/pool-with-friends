{
  description = "Pool with More Than Friends development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
        wasmBindgenTarget =
          {
            "aarch64-darwin" = {
              archive = "aarch64-apple-darwin";
              hash = "sha256-zZPmketZU6zl2P/OUqILAkB3o9rD4iFbgiQTaw77dYU=";
            };
            "aarch64-linux" = {
              archive = "aarch64-unknown-linux-musl";
              hash = "sha256-aZ3btyTs4W+RK3Opi1NBp19LjHiGzhUtQNvgn2lORsc=";
            };
            "x86_64-darwin" = {
              archive = "x86_64-apple-darwin";
              hash = "sha256-gQScefTig+FyXmWCoFKK8TAacK0jrdHfHE0ELsglJj0=";
            };
            "x86_64-linux" = {
              archive = "x86_64-unknown-linux-musl";
              hash = "sha256-YdSn3IWs+g0jVMzAuDYZKMflKnRtF/KOuqeV7T3BYUo=";
            };
          }
          .${system};
        wasmBindgenCli = pkgs.stdenvNoCC.mkDerivation {
          pname = "wasm-bindgen-cli";
          version = "0.2.127";
          src = pkgs.fetchurl {
            url = "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/0.2.127/wasm-bindgen-0.2.127-${wasmBindgenTarget.archive}.tar.gz";
            inherit (wasmBindgenTarget) hash;
          };
          sourceRoot = ".";
          installPhase = ''
            runHook preInstall
            mkdir -p "$out/bin"
            cp wasm-bindgen-0.2.127-${wasmBindgenTarget.archive}/{wasm-bindgen,wasm-bindgen-test-runner,wasm2es6js} "$out/bin/"
            runHook postInstall
          '';
        };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rustToolchain
            binaryen
            cargo-deny
            cargo-machete
            cargo-nextest
            curl
            geckodriver
            llvmPackages.bintools
            nodejs
            openssl
            pkg-config
            wasmBindgenCli
          ];

          shellHook = ''
            echo "Pool with More Than Friends development environment loaded"
            echo "  $(cargo --version)"
            echo "  $(rustc --version)"
            echo "  $(cargo nextest --version)"
            echo "  $(cargo machete --version)"
            echo "  $(cargo deny --version)"
            echo "  $(wasm-bindgen --version)"
            echo "  $(geckodriver --version | head -n 1)"

            if [ -z "$IN_NIX_SHELL_FISH" ] && [ -z "$BASH_EXECUTION_STRING" ]; then
              case "$-" in
                *i*) export IN_NIX_SHELL_FISH=1; exec fish ;;
              esac
            fi
          '';
        };
      }
    );
}
