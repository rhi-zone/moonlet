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

        # Common build inputs for all Rust packages
        commonNativeBuildInputs = with pkgs; [ pkg-config ];
        commonBuildInputs = with pkgs; [ openssl ];

        # Shared cargo lock configuration
        cargoLockConfig = {
          lockFile = ./Cargo.lock;
          outputHashes = {
            "rhizome-moss-0.1.0" = "sha256-3H+oEHhQ4OtiTMACEiH5pSKws/aUF9Nm2tombqUiGbg=";
            "rhizome-pith-filesystem-0.1.0" = "sha256-XD+/vftxHNrbt3lgJRUA8kr89hIDfVEs63kw5JbZER4=";
          };
        };

        # Helper function to build a plugin
        mkPlugin = { name, extraBuildInputs ? [] }: pkgs.rustPlatform.buildRustPackage {
          pname = "spore-${name}";
          version = "0.1.0";
          src = ./.;
          cargoLock = cargoLockConfig;
          nativeBuildInputs = commonNativeBuildInputs;
          buildInputs = commonBuildInputs ++ extraBuildInputs;
          cargoBuildFlags = [ "--package" "rhizome-spore-${name}" ];
          # Only install the shared library
          installPhase = ''
            runHook preInstall
            mkdir -p $out/lib/spore/plugins
            # Install plugin shared library - check CARGO_TARGET_DIR if set, else target/release
            libname="librhizome_spore_${builtins.replaceStrings ["-"] ["_"] name}"
            targetDir="''${CARGO_TARGET_DIR:-target}/release"
            for ext in so dylib dll; do
              if [ -f "$targetDir/$libname.$ext" ]; then
                cp "$targetDir/$libname.$ext" "$out/lib/spore/plugins/"
              fi
            done
            runHook postInstall
          '';
        };

        # Define all plugins
        plugins = {
          spore-embed = mkPlugin { name = "embed"; };
          spore-fs = mkPlugin { name = "fs"; };
          spore-libsql = mkPlugin { name = "libsql"; };
          spore-llm = mkPlugin { name = "llm"; };
          spore-moss = mkPlugin { name = "moss"; };
          spore-packages = mkPlugin { name = "packages"; };
          spore-sessions = mkPlugin { name = "sessions"; };
          spore-tools = mkPlugin { name = "tools"; };
        };

        # Core spore package (binary only)
        spore = pkgs.rustPlatform.buildRustPackage {
          pname = "spore";
          version = "0.1.0";
          src = ./.;
          cargoLock = cargoLockConfig;
          nativeBuildInputs = commonNativeBuildInputs;
          buildInputs = commonBuildInputs;
          cargoBuildFlags = [ "--package" "rhizome-spore" ];
        };

        # Combined package with core + all plugins
        spore-full = pkgs.symlinkJoin {
          name = "spore-full-0.1.0";
          paths = [ spore ] ++ (builtins.attrValues plugins);
          postBuild = ''
            # Ensure plugins directory exists in the combined output
            mkdir -p $out/lib/spore/plugins
          '';
        };

      in
      {
        packages = plugins // {
          default = spore;
          inherit spore spore-full;
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
