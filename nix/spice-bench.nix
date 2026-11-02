# The spice-bench suites (VAMS-Conformance-Suite `external/spice-bench`):
# its fetch.sh downloads every suite at pinned revisions and converts it to
# ngspice decks. That needs the network, so it runs in a fixed-output
# derivation whose hash pins the whole tree. The converted decks carry
# absolute `.include` paths into the tree; a fixed output cannot name its
# own store path, so they are written as @SPICE_BENCH@ and filled in by a
# second, ordinary derivation.
{ pkgs, conformance }:

let
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
''
