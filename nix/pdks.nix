# The sky130 and gf180mcu PDKs the examples run against, laid out as a ciel
# PDK_ROOT (`sky130A/`, `gf180mcuD/`, ...). Only the archives the ngspice
# model libraries need: each family's `common` (libs.tech) and its primitive
# devices (`*_fd_pr`, libs.ref). The versions are the ciel releases
# (fossi-foundation/ciel-releases), pinned by content hash.
{ pkgs }:

let
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
''
