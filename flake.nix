{
  description = "hivegame-uhp-api flake to setup everything";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        hive-hydra = pkgs.callPackage ./default.nix { };
        nokamute = pkgs.callPackage (pkgs.fetchFromGitHub {
          owner = "frisoft";
          repo = "nokamute";
          rev = "master";
          sha256 = "sha256-7Q2VuVexug0iqBXEzHfQ/c9q7TfjL56psGbq5sU2Nw4=";
        }) {};
      in
      with pkgs;
      {
        packages.default = hive-hydra;
        
        devShells.default = mkShell rec {
          buildInputs = [
            cargo
            rustfmt
            clippy
            rust-analyzer
            nokamute # use nokamute as AI during development
          ] ++ hive-hydra.buildInputs;
        };
      }
    );
}
