{ lib, rustPlatform, pkg-config, stdenv, darwin }:

rustPlatform.buildRustPackage rec {
  pname = "hive-hydra";
  version = "0.1.0";

  src = ./.;

  cargoLock = { lockFile = ./Cargo.lock; };

  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ ] ++ lib.optionals stdenv.isDarwin [
    darwin.apple_sdk.frameworks.SystemConfiguration
    darwin.apple_sdk.frameworks.CoreServices
    darwin.apple_sdk.frameworks.Security
  ];

  meta = with lib; {
    description = "Hive Hydra is a multi-bot and muti-AI client for hivegame.com";
    license = licenses.agpl;
    maintainers = with maintainers; [ "frisoft" ];
  };
}
