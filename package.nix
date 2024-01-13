{
  stdenv,
  rustPlatform,
  lib,
}: let
  inherit (lib.sources) sourceByRegex;
  src = sourceByRegex ./. ["Cargo.*" "(src|templates|benches)(/.*)?" "sqlx-data.json"];
in
  rustPlatform.buildRustPackage rec {
    pname = "dropstf";
    version = "0.1.0";

    inherit src;

    cargoLock = {
      lockFile = ./Cargo.lock;
    };

    SQLX_OFFLINE = true;
  }
