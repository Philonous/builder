{
  description = "Experimental build system";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.flake-utils.url = "github:numtide/flake-utils";

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        deps = [ pkgs.rustc
                 pkgs.cargo
                 pkgs.cargo-insta

                 pkgs.go-task
                 pkgs.rust-analyzer
                 pkgs.gdb
                 pkgs.rustfmt
               ] ;

      in
      {

        devShells.default = pkgs.mkShell {
          buildInputs = deps;
        };
      }
    );
}
