{
  description = "A rust based installer for TTW and MPI files. Built with Rust, and with speed in mind.";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    inherit (nixpkgs) lib;

    forAllSystems = lib.genAttrs [
      "x86_64-linux"
      "aarch64-linux"
    ];
  in {
    packages = forAllSystems (system: {
      default = nixpkgs.legacyPackages.${system}.callPackage ./package.nix {};
    });

    apps = forAllSystems (system: {
      cli = {
        type = "app";
        program = lib.getExe' self.packages.${system}.default "mpi_installer";
        meta.description = "Command line interface of the installer";
      };
    });

    devShells = forAllSystems (system: let
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      default = pkgs.mkShell {
        inputsFrom = [self.packages.${system}.default];

        packages = with pkgs; [
          clippy
          rust-analyzer
          rustfmt
        ];

        # cargo leaves the binary unpatched, so these come from the environment.
        LD_LIBRARY_PATH = lib.makeLibraryPath self.packages.${system}.default.runtimeLibs;
      };
    });
  };
}
