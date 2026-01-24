{
  description = "moonlet - Agentic AI framework with Lua scripting";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # Common build inputs for all Rust packages
        commonNativeBuildInputs = with pkgs; [ pkg-config ];
        commonBuildInputs = with pkgs; [ openssl ];

        # Shared cargo lock configuration
        cargoLockConfig = {
          lockFile = ./Cargo.lock;
          outputHashes = {
            "normalize-0.1.0" = "sha256-xDO5uDPLpZbdtOyTZnPUZv7247XVjD1e7bc0v3w+3YA=";
            "portals-filesystem-0.1.0" = "sha256-gj8ExV27uJ+e+hXob0+EIU/UOB62YfvxaaI+24wjbuM=";
          };
        };

        # Helper function to build a plugin
        mkPlugin = { name, extraBuildInputs ? [] }: pkgs.rustPlatform.buildRustPackage {
          pname = "moonlet-${name}";
          version = "0.1.0";
          src = ./.;
          cargoLock = cargoLockConfig;
          nativeBuildInputs = commonNativeBuildInputs;
          buildInputs = commonBuildInputs ++ extraBuildInputs;
          cargoBuildFlags = [ "--package" "moonlet-${name}" ];
          # Only install the shared library
          installPhase = ''
            runHook preInstall
            mkdir -p $out/lib/moonlet/plugins
            # Install plugin shared library
            # rustPlatform uses --target, so look in target/<triple>/release
            libname="libmoonlet_${builtins.replaceStrings ["-"] ["_"] name}"
            for targetDir in target/*/release target/release; do
              for ext in so dylib dll; do
                if [ -f "$targetDir/$libname.$ext" ]; then
                  cp "$targetDir/$libname.$ext" "$out/lib/moonlet/plugins/"
                fi
              done
            done
            runHook postInstall
          '';
        };

        # Define all plugins
        plugins = {
          moonlet-embed = mkPlugin { name = "embed"; };
          moonlet-fs = mkPlugin { name = "fs"; };
          moonlet-libsql = mkPlugin { name = "libsql"; };
          moonlet-llm = mkPlugin { name = "llm"; };
          moonlet-normalize = mkPlugin { name = "normalize"; };
          moonlet-packages = mkPlugin { name = "packages"; };
          moonlet-sessions = mkPlugin { name = "sessions"; };
          moonlet-tools = mkPlugin { name = "tools"; };
        };

        # Core moonlet package (binary only)
        moonlet = pkgs.rustPlatform.buildRustPackage {
          pname = "moonlet";
          version = "0.1.0";
          src = ./.;
          cargoLock = cargoLockConfig;
          nativeBuildInputs = commonNativeBuildInputs;
          buildInputs = commonBuildInputs;
          cargoBuildFlags = [ "--package" "moonlet" ];
        };

        # Combined package with core + all plugins
        moonlet-full = pkgs.symlinkJoin {
          name = "moonlet-full-0.1.0";
          paths = [ moonlet ] ++ (builtins.attrValues plugins);
          postBuild = ''
            # Ensure plugins directory exists in the combined output
            mkdir -p $out/lib/moonlet/plugins
          '';
        };

      in
      {
        packages = plugins // {
          default = moonlet;
          inherit moonlet moonlet-full;
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
