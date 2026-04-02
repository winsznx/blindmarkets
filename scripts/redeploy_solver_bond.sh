#!/bin/bash
# Redeploy SolverBond with STRK as bond token (18 decimals, minimum bond = 1 STRK)
# then rewire BatchAuction + BatchSettlement to point at the new contract.
set -e

cd "$(dirname "$0")"

if [ -f .env ]; then
    set -a; source .env; set +a
else
    echo "ERROR: .env not found"; exit 1
fi

ACCOUNT_NAME="bm_deployer"

# STRK on Sepolia
STRK_ADDRESS="0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"
# 1 STRK = 10^18 (low high)
MIN_BOND_LOW="1000000000000000000"
MIN_BOND_HIGH="0"

BATCH_AUCTION_ADDRESS="0x06302e0e4dd9953543cee26b3b0b8b15c324e927257251833c3bf0df36a9e1ae"
BATCH_SETTLEMENT_ADDRESS="0x06f7e4ea6102a0a3e9353041e59f3926da4fa7e24d89deff9a4173ad69b77c4e"
ZERO_ADDRESS="0x0000000000000000000000000000000000000000000000000000000000000000"

is_transient_error() {
    echo "$1" | grep -qi "cu limit exceeded\|request too fast\|error sending request\|connection refused\|timed out\|is not declared"
}

declare_contract() {
    local name=$1
    local json attempt wait_sec=15
    for attempt in 1 2 3 4 5; do
        json=$(sncast -j --account "$ACCOUNT_NAME" declare \
            --network sepolia \
            --contract-name "$name" 2>&1)
        echo "[declare $name]: $json" >&2
        if is_transient_error "$json"; then
            sleep $wait_sec; wait_sec=$((wait_sec + 15)); continue
        fi
        break
    done
    local hash
    hash=$(echo "$json" | /usr/bin/sed -n \
        's/.*"class_hash":"\(0x[0-9a-fA-F]*\)".*/\1/p; s/.*class hash \(0x[0-9a-fA-F]*\).*/\1/p' \
        | head -1)
    [ -z "$hash" ] && { echo "ERROR: could not extract class hash" >&2; exit 1; }
    if echo "$json" | grep -q '"command":"declare".*"transaction_hash"'; then
        echo "  (waiting 45s...)" >&2; sleep 45
    fi
    echo "$hash"
}

deploy_contract() {
    local class_hash=$1; shift
    local json attempt wait_sec=15
    for attempt in 1 2 3 4 5; do
        json=$(sncast -j --account "$ACCOUNT_NAME" deploy \
            --network sepolia \
            --class-hash "$class_hash" \
            --constructor-calldata "$@" 2>&1)
        echo "[deploy]: $json" >&2
        if is_transient_error "$json"; then
            sleep $wait_sec; wait_sec=$((wait_sec + 15)); continue
        fi
        break
    done
    local addr
    addr=$(echo "$json" | /usr/bin/sed -n 's/.*"contract_address":"\(0x[0-9a-fA-F]*\)".*/\1/p')
    [ -z "$addr" ] && { echo "ERROR: could not extract address" >&2; exit 1; }
    echo "$addr"
}

invoke_contract() {
    local address=$1 function=$2; shift 2
    local json attempt wait_sec=15
    for attempt in 1 2 3 4 5; do
        json=$(sncast -j --account "$ACCOUNT_NAME" invoke \
            --network sepolia \
            --contract-address "$address" \
            --function "$function" \
            --calldata "$@" 2>&1)
        echo "$json" >&2
        if is_transient_error "$json"; then
            sleep $wait_sec; wait_sec=$((wait_sec + 15)); continue
        fi
        break
    done
    echo "$json"
}

echo "== Step 1: Declare SolverBond =="
SOLVER_BOND_CLASS_HASH=$(declare_contract "SolverBond")
echo "  Class hash: $SOLVER_BOND_CLASS_HASH"

echo ""
echo "== Step 2: Deploy SolverBond (bond token = STRK, min bond = 1 STRK) =="
# constructor args match deploy_contracts.sh order:
#   admin, batch_auction (zero for now), batch_settlement (zero for now),
#   bond_token, treasury,
#   minimum_bond (u256 low high), withdrawal_delay_seconds,
#   slash_window_seconds, max_slashes_before_blacklist,
#   base_reputation (u256 low high), min_attempts_for_reputation,
#   reputation_decay_period,
#   slashing_user_bps, slashing_treasury_bps, slashing_whistleblower_bps,
#   bond_scaling_threshold (u256 low high), bond_scaling_percentage (u256 low high),
#   minimum_bond_update_delay
NEW_SOLVER_BOND_ADDRESS=$(deploy_contract "$SOLVER_BOND_CLASS_HASH" \
    "$ADMIN_ADDRESS" \
    "$ZERO_ADDRESS" \
    "$ZERO_ADDRESS" \
    "$STRK_ADDRESS" \
    "$TREASURY_ADDRESS" \
    "$MIN_BOND_LOW" "$MIN_BOND_HIGH" \
    "3600" \
    "3600" \
    "3" \
    "100" "0" \
    "5" \
    "604800" \
    "5000" "4000" "1000" \
    "10000000000000000000" "0" \
    "10000" "0" \
    "3600")
echo "  New SolverBond: $NEW_SOLVER_BOND_ADDRESS"

echo ""
echo "== Step 3: Wire SolverBond <-> BatchAuction + BatchSettlement =="
sleep 15
invoke_contract "$NEW_SOLVER_BOND_ADDRESS" "set_batch_auction_contract" "$BATCH_AUCTION_ADDRESS"
sleep 10
invoke_contract "$NEW_SOLVER_BOND_ADDRESS" "set_batch_settlement_contract" "$BATCH_SETTLEMENT_ADDRESS"
sleep 10
invoke_contract "$BATCH_AUCTION_ADDRESS" "set_solver_bond_contract" "$NEW_SOLVER_BOND_ADDRESS"
sleep 10
invoke_contract "$BATCH_SETTLEMENT_ADDRESS" "set_solver_bond_contract" "$NEW_SOLVER_BOND_ADDRESS"

echo ""
echo "========================================"
echo "Done! Update these values everywhere:"
echo "  SOLVER_BOND_ADDRESS=$NEW_SOLVER_BOND_ADDRESS"
echo ""
echo "Railway env vars to update:"
echo "  SOLVER_BOND_ADDRESS=$NEW_SOLVER_BOND_ADDRESS"
echo ""
echo "Update deployment_addresses.env and frontend .env.local too."
echo "========================================"
