{
  description = "kohiro - tiny git server with SSH TUI and CI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            go
            gopls
            gotools      # goimports, etc.
            git
            git-bug
            just
            sqlite       # debug queries against kohiro.db
          ];

          shellHook = ''
            export GOPATH="$PWD/.gopath"
            export PATH="$GOPATH/bin:$PATH"
          '';
        };
      }
    );
}
