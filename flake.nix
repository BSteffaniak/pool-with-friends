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
        rustToolchain = pkgs.rust-bin.stable."1.97.1".minimal.override {
          extensions = [
            "clippy"
            "rustfmt"
          ];
          targets = [ "wasm32-unknown-unknown" ];
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
            llvmPackages.bintools
            pkg-config
            wasm-bindgen-cli
          ];

          shellHook = ''
            echo "Pool with More Than Friends development environment loaded"
            echo "  $(cargo --version)"
            echo "  $(rustc --version)"
            echo "  $(cargo nextest --version)"
            echo "  $(cargo machete --version)"
            echo "  $(cargo deny --version)"

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
