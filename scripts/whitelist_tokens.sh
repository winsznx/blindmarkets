#!/bin/bash
set -e

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT/scripts"

if [ ! -f .env ]; then
  echo "ERROR: scripts/.env not found. Run ./scripts/setup_cairo.sh first."
  exit 1
fi

set -a; source .env; set +a

if [ -z "$DEPLOYER_ADDRESS" ] || [ -z "$DEPLOYER_PRIVATE_KEY" ]; then
  echo "ERROR: DEPLOYER_ADDRESS and DEPLOYER_PRIVATE_KEY must be set in scripts/.env"
  exit 1
fi

ASSET_REGISTRY="0x061b3c120cd166e1c523509a1ee153060b8afeab4f534c28d55d24de61bd4246"
ACCOUNT_NAME="bm_deployer"

# Import deployer account (idempotent)
sncast account import \
  --name "$ACCOUNT_NAME" \
  --address "$DEPLOYER_ADDRESS" \
  --private-key "$DEPLOYER_PRIVATE_KEY" \
  --type oz \
  --network sepolia 2>/dev/null || true

echo "Whitelisting tokens in AssetRegistry ($ASSET_REGISTRY)..."
echo ""

# ── USDC (Starkgate Sepolia) ─────────────────────────────────────────────────
# Symbol: USDC as felt252 = 0x55534443
USDC="0x053b40a647cedfca6ca84f542a0fe36736031905a9639a7f19a3c1e66bfd5080"
echo "Whitelisting USDC ($USDC)..."
sncast invoke \
  --account "$ACCOUNT_NAME" \
  --contract-address "$ASSET_REGISTRY" \
  --function whitelist_asset \
  --calldata "$USDC" "0x55534443" "6" \
  --network sepolia
echo "✓ USDC whitelisted"
echo ""

# ── WBTC ─────────────────────────────────────────────────────────────────────
# Set WBTC_ADDRESS in scripts/.env once you have the Starknet Sepolia address.
# Bridge from Ethereum Sepolia at https://sepolia.starkgate.starknet.io
if [ -n "$WBTC_ADDRESS" ]; then
  # Symbol: WBTC as felt252 = 0x57425443
  echo "Whitelisting WBTC ($WBTC_ADDRESS)..."
  sncast invoke \
    --account "$ACCOUNT_NAME" \
    --contract-address "$ASSET_REGISTRY" \
    --function whitelist_asset \
    --calldata "$WBTC_ADDRESS" "0x57425443" "8" \
    --network sepolia
  echo "✓ WBTC whitelisted"
else
  echo "WBTC_ADDRESS not set in scripts/.env — skipping WBTC."
  echo "Bridge WBTC at https://sepolia.starkgate.starknet.io then set WBTC_ADDRESS."
fi

echo ""
echo "Done. Verify with:"
echo "  sncast call --contract-address $ASSET_REGISTRY --function is_whitelisted --calldata $USDC --network sepolia"
