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

# New AssetRegistry from 2026-04-01 redeploy — admin is DEPLOYER_ADDRESS
ASSET_REGISTRY="${ASSET_REGISTRY_ADDRESS:-0x0676083bbbb6af43f48458e7fdab60d13a97f305b355f46da42f390dbd5eed55}"
ACCOUNT_NAME="bm_deployer"

echo "Using AssetRegistry: $ASSET_REGISTRY"
echo ""

echo "Importing deployer account..."
sncast account import \
  --name "$ACCOUNT_NAME" \
  --address "$DEPLOYER_ADDRESS" \
  --private-key "$DEPLOYER_PRIVATE_KEY" \
  --type ready \
  --network sepolia 2>/dev/null || true

invoke_or_fail() {
  local label="$1"; shift
  echo "Whitelisting $label..."
  output=$(sncast -j --account "$ACCOUNT_NAME" invoke --network sepolia "$@" 2>&1) || true
  if echo "$output" | grep -q '"type":"error"'; then
    echo "ERROR whitelisting $label:"
    echo "$output" | python3 -c "import sys,json; e=json.loads(sys.stdin.read()); print(e.get('error','unknown'))" 2>/dev/null || echo "$output"
    exit 1
  fi
  tx=$(echo "$output" | python3 -c "import sys,json; [print(json.loads(l).get('transaction_hash','')) for l in sys.stdin if 'transaction_hash' in l]" 2>/dev/null)
  echo "✓ $label whitelisted — tx: $tx"
  echo ""
}

USDC="0x053b40a647cedfca6ca84f542a0fe36736031905a9639a7f19a3c1e66bfd5080"
STRK="0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"

invoke_or_fail "USDC" \
  --contract-address "$ASSET_REGISTRY" \
  --function whitelist_asset \
  --calldata "$USDC" "0x55534443" "6"

invoke_or_fail "STRK" \
  --contract-address "$ASSET_REGISTRY" \
  --function whitelist_asset \
  --calldata "$STRK" "0x5354524b" "18"

if [ -n "$WBTC_ADDRESS" ]; then
  invoke_or_fail "WBTC" \
    --contract-address "$ASSET_REGISTRY" \
    --function whitelist_asset \
    --calldata "$WBTC_ADDRESS" "0x57425443" "8"
else
  echo "WBTC_ADDRESS not set — skipping."
fi

echo "Verifying..."
for token in "$USDC" "$STRK"; do
  result=$(sncast -j call --network sepolia \
    --contract-address "$ASSET_REGISTRY" \
    --function is_whitelisted \
    --calldata "$token" 2>/dev/null | python3 -c "import sys,json; d=[json.loads(l) for l in sys.stdin]; r=[x for x in d if 'response' in x]; print(r[0]['response'][0] if r else '?')" 2>/dev/null)
  echo "$token => whitelisted: $result"
done
