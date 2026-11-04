{
  description = "SpiceRack: SpiceRack core rewritten in Rust";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    # ESPice, built from source: the oracle `espice` the dialect and backend
    # tests run.
    espice.url = "git+https://github.com/OpenFAT/ESPice.git?shallow=1";
    espice.inputs.conformance.follows = "conformance";
    # The VAMS conformance suite: its HSPICE decks and spice-bench's fetch.sh.
    conformance.url = "github:OpenFAT/VAMS-Conformance-Suite";
    conformance.inputs.nixpkgs.follows = "nixpkgs";
    conformance.inputs.flake-utils.follows = "flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, espice, conformance }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [
          (import rust-overlay)
          # Darwin needs llvmPackages_18 stdenv for ngspice to build.
          # No version pin — nixpkgs-unstable tracks ngspice 44+ which
          # supports OSDI v0.4 modules from openvaf-r.  The binned-model
          # scoped-resolution issue is a fundamental ngspice
          # limitation present in all versions, not a 44-specific regression.
          (final: prev: {
            ngspice = if prev.stdenv.isDarwin
              then prev.ngspice.override { stdenv = prev.llvmPackages_18.stdenv; }
              else prev.ngspice;
          })
        ];
        pkgs = import nixpkgs { inherit system overlays; };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
        pythonEnv = pkgs.python312.withPackages (ps: with ps; [
          numpy
          pytest
        ]);
        openvaf = pkgs.rustPlatform.buildRustPackage rec {
          pname = "openvaf-r";
          version = "unstable-2026";

          src = pkgs.fetchFromGitHub {
            owner = "arpadbuermen";
            repo = "OpenVAF";
            rev = "2e066436d985b05cf8e6563e936daf9ab875775a";
            hash = "sha256-AXtp8qaDq/MRYz2TYXRwT3kS+8EnKyakD3lQwdv3K34=";
          };

          cargoLock = {
            lockFile = "${src}/Cargo.lock";
            outputHashes = {
              "salsa-0.17.0-pre.2" = "sha256-6GssvV76lFr5OzAUekz2h6f82Tn7usz5E8MSZ5DmgJw=";
            };
          };

          # Patch build scripts to skip Windows-only steps on Linux hosts.
          # RUST_CHECK was used previously for this but it also skips osdi stdlib.c bitcode
          # generation, producing a broken binary. Instead patch each build.rs individually.
          postPatch = ''
            # target/build.rs: skip MSVC ucrt import-lib on non-Windows host
            sed -i 's/if check {/if check || !cfg!(target_os = "windows") {/g' \
              openvaf/target/build.rs
            # osdi/build.rs: skip generating bitcode for MSVC targets on non-Windows host
            sed -i 's/if no_gen {/if no_gen || (target.options.is_like_windows \&\& !cfg!(target_os = "windows")) {/' \
              openvaf/osdi/build.rs
          '';

          buildAndTestSubdir = "openvaf/openvaf-driver";

          buildFeatures = [ "llvm18" ];

          nativeBuildInputs = with pkgs; [ pkg-config ];

          buildInputs = with pkgs; [
            llvm_18
            libffi
            libxml2
            zlib
          ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.libiconv
          ];

          # symlinkJoin provides both llvm-config (for llvm-sys) AND unwrapped clang
          # (for osdi/build.rs stdlib.c -> bitcode cross-compilation).
          # The wrapped clang adds x86_64-specific Nix flags that break -target riscv64 etc.
          env.LLVM_SYS_181_PREFIX = "${pkgs.symlinkJoin {
            name = "llvm18-prefix";
            paths = [
              pkgs.llvm_18.dev
              pkgs.llvmPackages_18.clang-unwrapped
            ];
          }}";

          doCheck = false;

          # Only install the openvaf-r binary
          postInstall = ''
            find $out/bin -type f ! -name "openvaf-r" -delete 2>/dev/null || true
          '';

          meta = {
            description = "OpenVAF-reloaded: Verilog-A compiler for VACASK";
            homepage = "https://github.com/arpadbuermen/OpenVAF";
            license = pkgs.lib.licenses.gpl3Only;
            platforms = pkgs.lib.platforms.linux ++ pkgs.lib.platforms.darwin;
          };
        };

        vacask = pkgs.stdenv.mkDerivation rec {
          pname = "vacask";
          version = "unstable-2026";

          src = pkgs.fetchFromGitHub {
            owner = "robtaylor";
            repo = "VACASK";
            rev = "bcd48e2dd25182f5aaa3392c4e27b4e198372744";
            hash = "sha256-/x6yJ+fklipvYbtI5rHx4d5YIpC9IJ5uhHCtWC5eJJg=";
          };

          nativeBuildInputs = with pkgs; [
            cmake
            ninja
            pkg-config
            python3
            bison
            flex
          ];

          buildInputs = with pkgs; [
            suitesparse
            openblas
            boost
            tomlplusplus
          ];

          postPatch = ''
            # --- Bypass all Darwin/Homebrew blocks ---
            # The CMakeLists.txt "Darwin" blocks (lines 47, 130, 280) assume Homebrew:
            # they call brew --prefix, set SuiteSparse_DIR/TOMLPP_DIR/Boost_ROOT to
            # Homebrew paths, etc. None of this works in Nix's sandbox.
            # By renaming "Darwin" the platform falls through to the Linux/generic
            # else blocks, where all our Nix patches already apply.
            sed -i 's/STREQUAL "Darwin"/STREQUAL "DarwinBrew"/g' CMakeLists.txt

            # --- Linux/generic block Boost fixes ---
            sed -i 's/set(Boost_NO_SYSTEM_PATHS TRUE)//' CMakeLists.txt
            sed -i 's/find_package(Boost 1.88 REQUIRED COMPONENTS filesystem process system)/find_package(Boost REQUIRED COMPONENTS filesystem process)/' CMakeLists.txt
            sed -i 's|set(Boost_EXTRA_LINK_DIR "''${Boost_INCLUDE_DIRS}/stage/lib")|set(Boost_EXTRA_LINK_DIR "''${Boost_LIBRARY_DIRS}")|' CMakeLists.txt
            sed -i 's/boost_system boost_filesystem boost_process/boost_filesystem boost_process/' CMakeLists.txt

            # --- Compiler fixes ---
            # Remove -fcoroutines (GCC-only flag, clang rejects it; C++20 coroutines
            # are enabled by default in both compilers anyway).
            sed -i 's/ "-fcoroutines"//' CMakeLists.txt

            # --- KLU type fix (macOS ARM64) ---
            # On macOS ARM64: int64_t = long long, SuiteSparse_long = long.
            # Both 64-bit but C++ treats them as different types.
            # Replace int64_t with SuiteSparse_long in KLU template instantiations.
            sed -i 's/int64_t/SuiteSparse_long/g' lib/klumatrix.cpp
            sed -i 's/int64_t/SuiteSparse_long/g' lib/klubsmatrix.cpp

            # --- Header fixes ---
            sed -i 's|suitesparse/klu.h|klu.h|g' include/klumatrix.h

            # --- Portable device models ---
            # OpenVAF compiles for the build machine's CPU by default ('native'). A
            # store path built on one CI runner and fetched from the cache onto a
            # runner with an older CPU then dies with SIGILL in every device model.
            sed -i 's|ARGS "''${options_list}" "--allow"|ARGS "''${options_list}" "--target_cpu" "generic" "--allow"|' devices/CMakeLists.txt
            grep -q -- '"--target_cpu" "generic"' devices/CMakeLists.txt
          '';

          # Suppress clang warnings that newer clang promotes to hard errors.
          env.NIX_CFLAGS_COMPILE = "-Wno-error -Wno-defaulted-function-deleted";

          cmakeFlags = [
            "-DCMAKE_BUILD_TYPE=Release"
            "-DOPENVAF_DIR=${openvaf}/bin"
            "-DTOMLPP_DIR=${pkgs.tomlplusplus}"
            "-DSuiteSparse_DIR=${pkgs.suitesparse}"
          ];

          # CMake's own install: bin/vacask plus lib/vacask/mod/**.osdi (the device
          # models OpenVAF compiled, e.g. spice/capacitor.osdi) and lib/vacask/config,
          # where the binary looks for them relative to itself. Copying only the
          # binary left every netlist that loads a device module unrunnable.

          meta = {
            description = "VACASK – Verilog-A Circuit Analysis Kernel";
            homepage = "https://github.com/robtaylor/VACASK";
            license = pkgs.lib.licenses.gpl2Plus;
            platforms = pkgs.lib.platforms.linux ++ pkgs.lib.platforms.darwin;
          };
        };

        # The spice-bench suites (VAMS-Conformance-Suite `external/spice-bench`):
        # its fetch.sh downloads every suite at pinned revisions and converts it to
        # ngspice decks. That needs the network, so it runs in a fixed-output
        # derivation whose hash pins the whole tree. The converted decks carry
        # absolute `.include` paths into the tree; a fixed output cannot name its
        # own store path, so they are written as @SPICE_BENCH@ and filled in by a
        # second, ordinary derivation.
        spiceBench = let
          fetched = pkgs.stdenvNoCC.mkDerivation {
            name = "spice-bench-fetched";
            src = "${conformance}/external/spice-bench";
            nativeBuildInputs = with pkgs; [
              bzip2
              cacert
              curl
              gawk
              git
              gnutar
              gzip
              p7zip
              python3
              unzip
              zstd
            ];
            dontConfigure = true;
            dontInstall = true;
            dontFixup = true;
            buildPhase = ''
              cp -r $src sb && chmod -R u+w sb && patchShebangs sb
              # The scripts call `nix shell nixpkgs#X -c`; X is on PATH here.
              sed -i 's/nix shell nixpkgs#[a-z0-9]* -c //' sb/*/fetch.sh
              export HOME=$TMPDIR SSL_CERT_FILE=${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt
              sb/fetch.sh "$TMPDIR/out"
              # Git metadata is not content: its packs differ run to run.
              find "$TMPDIR/out" -name .git -prune -exec rm -rf {} +
              grep -rlZ --binary-files=without-match "$TMPDIR/out" "$TMPDIR/out" | xargs -0 -r sed -i "s|$TMPDIR/out|@SPICE_BENCH@|g"
              mv "$TMPDIR/out" $out
            '';
            outputHashMode = "recursive";
            outputHash = "sha256-g3Gw1U9aqLcofpHteGPurGNGxxRo8UsPi49J+cqhT/s=";
          };
        in
        pkgs.runCommand "spice-bench" { } ''
          cp -r ${fetched} $out
          chmod -R u+w $out
          grep -rlZ --binary-files=without-match @SPICE_BENCH@ $out | xargs -0 -r sed -i "s|@SPICE_BENCH@|$out|g"
        '';

        # The sky130 and gf180mcu PDKs the examples run against, laid out as a ciel
        # PDK_ROOT (`sky130A/`, `gf180mcuD/`, ...). Only the archives the ngspice
        # model libraries need: each family's `common` (libs.tech) and its primitive
        # devices (`*_fd_pr`, libs.ref). The versions are the ciel releases
        # (fossi-foundation/ciel-releases), pinned by content hash.
        pdks = let
          release = tag: file: hash: pkgs.fetchurl {
            url = "https://github.com/fossi-foundation/ciel-releases/releases/download/${tag}/${file}";
            inherit hash;
          };
          sky130 = "sky130-8afc8346a57fe1ab7934ba5a6056ea8b43078e71";
          gf180 = "gf180mcu-1e3554706ddbb2529193bdabbc7128296f3cb44a";
          archives = [
            (release sky130 "common.tar.zst" "sha256-vopUg0H9kXevnkYa17T92Jr74D26aNxC0VUyH9w/hr8=")
            (release sky130 "sky130_fd_pr.tar.zst" "sha256-DOTQNwGnUO4NaX0DU2yy4+yJyMf0yjfRp3NtVGh0+uY=")
            (release gf180 "common.tar.zst" "sha256-WFQYetcapWoYSuypweZR/0ekSEHutUlS4VdU2qbspDA=")
            (release gf180 "gf180mcu_fd_pr.tar.zst" "sha256-wT2bul+z1FT/7n7q6yFdF5h8kMmBu/sLVyWGNrp8OMw=")
          ];
        in
        pkgs.runCommand "spicerack-pdks" { nativeBuildInputs = [ pkgs.zstd ]; } ''
          mkdir -p $out
          for a in ${pkgs.lib.concatStringsSep " " archives}; do
            zstd -dc "$a" | tar -x -C $out
          done
        '';
        # src/veriloga.rs runs `openvaf`; the reloaded fork installs `openvaf-r`.
        openvafAlias = pkgs.runCommand "openvaf-alias" { } ''
          mkdir -p $out/bin && ln -s ${openvaf}/bin/openvaf-r $out/bin/openvaf
        '';
        # Read-only deck corpora for tests/test_import_suite/'s fixpoints.
        xyceRegression = pkgs.fetchFromGitHub {
          owner = "Xyce";
          repo = "Xyce_Regression";
          rev = "bbde402756cdf542627ff7ea0056db130fcb005b";
          sparseCheckout = [ "Netlists" ];
          hash = "sha256-D+eOS9tMNZ9L/xVOuQnL2ahIO2zhIiv1YiucOWjAAng=";
        };
        spiceDatasets = pkgs.fetchFromGitHub {
          owner = "symbench";
          repo = "spice-datasets";
          rev = "a3a1c4fc3b8d3d5cd4e986b50e85bd5858751559";
          hash = "sha256-sf1d3jLmCPNg7yphMedOlCtGTUQCeyxPu4hdRdgzkQQ=";
        };

        srcFiltered = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            let baseName = baseNameOf (toString path); in
            !(baseName == "target" || baseName == "target_nix" || baseName == "result"
              || baseName == ".git");
        };

        spicerackRs = pkgs.python312Packages.buildPythonPackage {
          pname = "spicerack";
          version = "0.1.0";
          format = "pyproject";
          src = srcFiltered;

          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            name = "spicerack-vendor";
            hash = "sha256-v/ULmZ8HRut7EoEJLARkwNJcpURIuao5w7HTO8L2VKg=";
            src = srcFiltered;
          };

          nativeBuildInputs = [
            pkgs.rustPlatform.cargoSetupHook
            pkgs.rustPlatform.maturinBuildHook
            pkgs.cargo
            rustToolchain
            pkgs.pkg-config
          ];

          propagatedBuildInputs = [
            pkgs.python312Packages.numpy
          ];

          pythonImportsCheck = [ "spicerack" ];
        };
        # Build and unit-test shell. No simulators: SpiceRack emits netlists and
        # runs whatever simulator the user has installed; it never ships one.
        dev = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.maturin
            pythonEnv
            pkgs.pkg-config
          ];
          RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
          # ponytail: venv pip wheels (numpy) are manylinux binaries needing
          # libstdc++.so.6, which NixOS doesn't expose globally. Upgrade if
          # the venv ever moves fully onto nixpkgs' python package set.
          LD_LIBRARY_PATH = "${pkgs.stdenv.cc.cc.lib}/lib";
        };
      in {
        devShells.default = dev;

        # `nix develop .#testing`: dev shell plus the simulators and PDKs the
        # integration tests check our output against.
        devShells.testing = pkgs.mkShell {
          inputsFrom = [ dev ];
          packages = [
            pkgs.ngspice
            openvaf
            openvafAlias
            vacask
            espice.packages.${system}.espice
          ];
          # tests/test_import_suite/: the spice-bench round trip and the
          # per-dialect fixpoint corpora.
          SPICE_BENCH = spiceBench;
          NGSPICE_CORPUS = spiceBench;
          HSPICE_CORPUS = "${conformance}/tests/netlist";
          XYCE_CORPUS = "${xyceRegression}/Netlists";
          VACASK_CORPUS = vacask.src;
          LTSPICE_CORPUS = spiceDatasets;

          # examples 20 and 21: sky130 and gf180mcu, a ciel-layout PDK_ROOT.
          PDK_ROOT = pdks;
        };

        # `nix develop .#docs -c mkdocs build`: the documentation site from docs/.
        devShells.docs = pkgs.mkShell {
          packages = [ (pkgs.python312.withPackages (ps: [ ps.mkdocs ps.mkdocs-material ])) ];
        };

        packages.default = spicerackRs;
      }
    );
}
