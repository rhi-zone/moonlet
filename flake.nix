{
  description = "spore - Agentic AI framework with Lua scripting";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "spore";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ openssl ];
        };

        devShells.default = pkgs.mkShell rec {
          buildInputs = with pkgs; [
            stdenv.cc.cc
            # Rust toolchain
            rustc
            cargo
            rust-analyzer
            clippy
            rustfmt
            # Fast linker for incremental builds
            mold
            clang
            # System deps
            openssl
            pkg-config
            # JS tooling for docs
            bun
          ];
          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath buildInputs}:$LD_LIBRARY_PATH";
        };
      }
    );
}
