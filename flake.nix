{
  description = "url-shortener devshell (Rust + cargo-lambda + AWS CLI + SAM CLI via uv in .venv)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };

      # Pick a conservative Python version for tooling stability.
      # (Avoid following nixpkgs default python if it jumps to 3.13+ and breaks deps.)
      py = pkgs.python312;
    in {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          rustup
          cargo-lambda
          awscli2

          uv
          py

          openssl
          pkg-config
          cmake
          git
          jq
          zip
          unzip
        ];

        shellHook = ''
          set -euo pipefail

          # --- Rust toolchain (your existing approach) ---
          if ! command -v rustc >/dev/null 2>&1; then
            rustup toolchain install stable >/dev/null
            rustup default stable >/dev/null
          fi

          # --- Project-local Python tooling venv (SAM CLI) ---
          VENV_DIR="$PWD/.venv"
          export VIRTUAL_ENV="$VENV_DIR"
          export PATH="$VENV_DIR/bin:$PATH"

          # Create venv if missing.
          if [ ! -x "$VENV_DIR/bin/python" ]; then
            echo "[tools] Creating venv at $VENV_DIR"
            ${py}/bin/python -m venv "$VENV_DIR"
          fi

          # Install SAM CLI if missing.
          # NOTE: This does network I/O the first time. If you want "no network on direnv load",
          # move this into a Makefile target instead.
          if [ ! -x "$VENV_DIR/bin/sam" ]; then
            echo "[tools] Installing aws-sam-cli into $VENV_DIR via uv..."
            # Ensure install goes into our venv even if some other env is active
            uv pip install --python "$VENV_DIR/bin/python" --upgrade aws-sam-cli || {
              echo "[tools] WARNING: Failed to install aws-sam-cli (offline?)."
              echo "        Retry with: uv pip install --python $VENV_DIR/bin/python --upgrade aws-sam-cli"
            }
          fi

          echo "Dev shell ready."
          echo "  sam:   $(sam --version 2>/dev/null || echo 'not installed yet')"
          echo "  aws:   $(aws --version 2>/dev/null || true)"
        '';
      };
    };
}
