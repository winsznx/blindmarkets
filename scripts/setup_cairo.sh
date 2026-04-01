#!/bin/bash
set -e

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "BlindMarkets — Cairo toolchain setup"
echo "====================================="
echo ""

# ── 1. Install starkup (installs scarb + sncast in one shot) ─────────────────
if command -v scarb &>/dev/null && command -v sncast &>/dev/null; then
  echo "✓ scarb $(scarb --version | head -1) already installed"
  echo "✓ sncast $(sncast --version | head -1) already installed"
else
  echo "Installing starkup (scarb + sncast)..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.starkup.sh | sh

  # starkup writes to ~/.local/bin — add to PATH for the rest of this script
  export PATH="$HOME/.local/bin:$PATH"

  if ! command -v scarb &>/dev/null; then
    echo ""
    echo "✓ Installation done. Restart your terminal (or run: source ~/.zshrc)"
    echo "  then re-run this script to verify + build."
    exit 0
  fi

  echo "✓ scarb $(scarb --version | head -1)"
  echo "✓ sncast $(sncast --version | head -1)"
fi

echo ""

# ── 2. Build contracts ───────────────────────────────────────────────────────
echo "Building contracts..."
cd "$REPO_ROOT/contracts"
scarb build

echo ""
echo "✓ Build complete. Sierra artifacts:"
ls target/dev/*.contract_class.json 2>/dev/null | sed 's|.*/||'

echo ""

# ── 3. Set up scripts/.env if not present ───────────────────────────────────
cd "$REPO_ROOT/scripts"
if [ ! -f .env ]; then
  cp .env.example .env
  # Update defaults to Sepolia
  sed -i '' 's|STARKNET_RPC_URL=.*|STARKNET_RPC_URL=https://starknet-sepolia.public.blastapi.io/rpc/v0_7|' .env
  sed -i '' 's|STARKNET_CHAIN_ID=.*|STARKNET_CHAIN_ID=SN_SEPOLIA|' .env
  echo "✓ Created scripts/.env from template — fill in your deployer keys:"
  echo ""
  echo "  DEPLOYER_PRIVATE_KEY=0x..."
  echo "  DEPLOYER_ADDRESS=0x..."
  echo "  ADMIN_ADDRESS=0x...       (your wallet)"
  echo "  TREASURY_ADDRESS=0x...    (your wallet)"
  echo "  COORDINATOR_ADDRESS=0x... (your wallet or a fresh account)"
  echo "  BOND_TOKEN_ADDRESS=0x...  (USDC or WBTC on Sepolia)"
  echo ""
  echo "  Edit: $REPO_ROOT/scripts/.env"
else
  echo "✓ scripts/.env already exists"
fi

echo ""
echo "Next steps:"
echo "  1. Fill in scripts/.env (deployer private key, addresses)"
echo "  2. cd $REPO_ROOT && ./scripts/deploy_contracts.sh"
