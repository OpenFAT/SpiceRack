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
          (import ./nix/ngspice.nix)
        ];
        pkgs = import nixpkgs { inherit system overlays; };
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };
        pythonEnv = pkgs.python312.withPackages (ps: with ps; [
          numpy
          pytest
        ]);
        openvaf = import ./nix/openvaf.nix { inherit pkgs; };
        vacask = import ./nix/vacask.nix { inherit pkgs; openvafPkg = openvaf; };
        spiceBench = import ./nix/spice-bench.nix { inherit pkgs conformance; };
        pdks = import ./nix/pdks.nix { inherit pkgs; };
        # src/veriloga.rs runs `openvaf`; the reloaded fork installs `openvaf-r`.
        openvafAlias = pkgs.runCommand "openvaf-alias" { } ''
          mkdir -p $out/bin && ln -s ${openvaf}/bin/openvaf-r $out/bin/openvaf
        '';
        # Read-only deck corpora for tests/test_import_suite.rs's fixpoints.
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
          # tests/test_import_suite.rs: the spice-bench round trip and the
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
