'use client';

import { useEffect, useState } from 'react';
import { motion } from 'framer-motion';
import clsx from 'clsx';
import { RpcProvider } from 'starknet';
import { MAX_INTENT_DEADLINE_MINUTES, useIntentStore } from '../state/useIntentStore';
import { buildIntent, encryptIntentForGateway, generateNonce } from '../lib/intentCrypto';
import type { IntentPayload } from '../lib/intentCrypto';
import {
  buildCancelIntentCall,
  signIntentAuthorization,
  truncateAddress,
} from '../lib/starkzap-wallet';

const KNOWN_TOKENS: Record<string, { symbol: string; decimals: number }> = {
  '0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d': { symbol: 'STRK', decimals: 18 },
  '0x053b40a647cedfca6ca84f542a0fe36736031905a9639a7f19a3c1e66bfd5080': { symbol: 'USDC', decimals: 6 },
};

function tokenSymbol(address: string): string {
  const normalized = address.toLowerCase().replace(/^0x0*/, '0x');
  for (const [addr, info] of Object.entries(KNOWN_TOKENS)) {
    if (addr.toLowerCase().replace(/^0x0*/, '0x') === normalized) return info.symbol;
  }
  return truncateAddress(address);
}

async function fetchTokenBalance(tokenAddress: string, walletAddress: string): Promise<string> {
  const rpc = process.env.NEXT_PUBLIC_STARKNET_RPC ?? 'https://free-rpc.nethermind.io/sepolia-juno/rpc/v0_7';
  const provider = new RpcProvider({ nodeUrl: rpc });
  try {
    const result = await provider.callContract({
      contractAddress: tokenAddress,
      entrypoint: 'balanceOf',
      calldata: [walletAddress],
    });
    const raw = BigInt(result[0]);
    const token = Object.entries(KNOWN_TOKENS).find(([addr]) =>
      addr.toLowerCase().replace(/^0x0*/, '0x') === tokenAddress.toLowerCase().replace(/^0x0*/, '0x')
    );
    const decimals = token?.[1].decimals ?? 18;
    const divisor = 10n ** BigInt(decimals);
    const whole = raw / divisor;
    const frac = raw % divisor;
    const fracStr = frac.toString().padStart(decimals, '0').slice(0, 4).replace(/0+$/, '');
    return fracStr ? `${whole}.${fracStr}` : `${whole}`;
  } catch {
    return '—';
  }
}

const privacyOptions = [
  { id: 'public', label: 'Public', icon: '🔓', color: 'text-text-muted', description: 'Pair, size, and direction visible to solvers from submission.' },
  { id: 'hidden-amount', label: 'Hidden Amount', icon: '🔒', color: 'text-accent-warning', description: 'Pair visible, order size hidden until the batch closes.' },
  { id: 'hidden-direction', label: 'Hidden Direction', icon: '🔐', color: 'text-accent-success', description: 'Size and direction both hidden. Solver sees neither until execution.' },
] as const;

export default function IntentComposer() {
  const {
    draft,
    setDraft,
    wallet,
    walletAddress,
    walletProviderKey,
    setWalletSession,
  } = useIntentStore();
  const [privacy, setPrivacy] = useState(draft.privacyMode);
  const [intentHash, setIntentHash] = useState('');
  const [intentId, setIntentId] = useState('');
  const [preparedNonce, setPreparedNonce] = useState('');
  const [preparedIntent, setPreparedIntent] = useState<IntentPayload | null>(null);
  const [statusIntentId, setStatusIntentId] = useState('');
  const [statusResult, setStatusResult] = useState<string | null>(null);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isCheckingStatus, setIsCheckingStatus] = useState(false);
  const [isCanceling, setIsCanceling] = useState(false);
  const [assetInBalance, setAssetInBalance] = useState<string | null>(null);

  useEffect(() => {
    if (!walletAddress || !draft.assetIn) { setAssetInBalance(null); return; }
    setAssetInBalance(null);
    fetchTokenBalance(draft.assetIn, walletAddress).then(setAssetInBalance);
  }, [walletAddress, draft.assetIn]);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const minOutputValue = parseOptionalBigint(draft.minOutput);
  const amountValue = parseOptionalBigint(draft.amount);
  const minOutputCap = resolveMinOutputCap(amountValue, minOutputValue);
  const minOutputSliderValue =
    minOutputCap > 0n ? Number((minOutputValue * 1000n) / minOutputCap) : 0;
  const maxFeePercent = draft.maxFeeBps / 100;

  useEffect(() => {
    setPreparedNonce('');
    setPreparedIntent(null);
    setIntentHash('');
    setIntentId('');
  }, [
    draft.assetIn,
    draft.assetOut,
    draft.amount,
    draft.minOutput,
    draft.deadlineMinutes,
    draft.maxFeeBps,
    privacy,
    walletAddress,
  ]);

  const prepareIntent = () => {
    setStatusMessage(null);
    if (!walletAddress) {
      setStatusMessage('Connect a Starknet wallet before preparing an intent.');
      return;
    }
    if (!draft.assetIn || !draft.assetOut || !draft.amount || !draft.minOutput || !draft.deadlineMinutes) {
      setStatusMessage('Complete all intent fields before preparing.');
      return;
    }
    const deadlineError = validateDeadlineMinutes(draft.deadlineMinutes);
    if (deadlineError) {
      setStatusMessage(deadlineError);
      return;
    }
    if (!isHex(walletAddress) || !isHex(draft.assetIn) || !isHex(draft.assetOut)) {
      setStatusMessage('Addresses must be hex values starting with 0x.');
      return;
    }
    if (isZeroAddress(draft.assetIn) || isZeroAddress(draft.assetOut)) {
      setStatusMessage('Asset addresses cannot be the zero address. Enter the deployed token contract addresses.');
      return;
    }

    try {
      const amount = parseBigint(draft.amount);
      const minOut = parseBigint(draft.minOutput);
      const deadline = BigInt(Math.floor(Date.now() / 1000) + draft.deadlineMinutes * 60);
      const nonce = generateNonce();
      const intent = buildIntent({
        userAddress: walletAddress,
        assetIn: draft.assetIn,
        assetOut: draft.assetOut,
        amount,
        minOutput: minOut,
        maxFeeBps: draft.maxFeeBps,
        deadline,
        privacyMode: privacy,
        nonce,
      });
      setPreparedNonce(nonce);
      setPreparedIntent(intent);
      setIntentHash(intent.intentHash);
      setIntentId(intent.intentId);
      setStatusIntentId(intent.intentId);
      setStatusMessage('Intent prepared. Submit to store ciphertext, then approve the wallet transaction.');
    } catch (error) {
      setStatusMessage(`Preparation failed: ${String(error)}`);
    }
  };

  const onSubmit = async () => {
    setStatusMessage(null);
    if (!walletAddress) {
      setStatusMessage('Connect a Starknet wallet before submitting.');
      return;
    }
    if (!draft.assetIn || !draft.assetOut || !draft.amount || !draft.minOutput || !draft.deadlineMinutes) {
      setStatusMessage('Complete all intent fields before submitting.');
      return;
    }
    const deadlineError = validateDeadlineMinutes(draft.deadlineMinutes);
    if (deadlineError) {
      setStatusMessage(deadlineError);
      return;
    }
    if (!preparedNonce || !preparedIntent) {
      setStatusMessage('Prepare the intent before submitting.');
      return;
    }

    try {
      setIsSubmitting(true);
      const activeWallet = await ensureWalletSession(wallet);
      if (normalizeHex(activeWallet.address) !== normalizeHex(walletAddress)) {
        throw new Error('Connected wallet changed. Re-prepare the intent and try again.');
      }

      const intent = preparedIntent;
      setIntentHash(intent.intentHash);
      setIntentId(intent.intentId);
      setStatusIntentId(intent.intentId);

      const gatewayPublicKeyResponse = await fetch('/api/gateway/public-key');
      if (!gatewayPublicKeyResponse.ok) {
        throw new Error(await extractGatewayError(gatewayPublicKeyResponse));
      }
      const gatewayPublicKey = (await gatewayPublicKeyResponse.json()).gateway_public_key as string;

      const encrypted = await encryptIntentForGateway(intent, gatewayPublicKey);
      if (!activeWallet.signMessage) {
        throw new Error('Connected wallet does not support message signing.');
      }
      const authorization = await signIntentAuthorization(activeWallet as Required<typeof activeWallet>, intent);

      const storageResponse = await fetch('/api/gateway/intents', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          intent_id: intent.intentId,
          user_address: intent.userAddress,
          ciphertext: encrypted.ciphertextHex,
          encrypted_session_key: encrypted.encryptedSessionKeyHex,
          commitment: intent.intentHash,
          user_signature: authorization.signature,
          client_public_key: encrypted.clientPublicKeyHex,
          nonce: intent.nonce,
          authorization_hash: authorization.authorizationHash,
          submission_mode: 'SELF_COMMIT',
        }),
      });

      if (!storageResponse.ok && storageResponse.status !== 409) {
        throw new Error(await extractGatewayError(storageResponse));
      }

      const tx = await activeWallet.execute(
        [{
          contractAddress: process.env.NEXT_PUBLIC_INTENT_REGISTRY!,
          entrypoint: 'commit_intent',
          calldata: [intent.intentHash, '0x0', `0x${intent.deadline.toString(16)}`],
        }],
        { feeMode: 'sponsored' },
      );
      await tx.wait();
      const txHash = tx.hash;
      setPreparedNonce('');
      setPreparedIntent(null);

      const reconcileResponse = await fetch(`/api/gateway/intents/${intent.intentId}/onchain`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          action: 'COMMITTED',
          user_address: intent.userAddress,
          tx_hash: txHash,
        }),
      });

      if (!reconcileResponse.ok) {
        setStatusMessage(
          `Intent committed on-chain. Gateway reconciliation is pending via the observer. Tx ${truncateAddress(txHash)}`
        );
        return;
      }

      setStatusMessage(`Intent submitted and committed on-chain. Tx ${truncateAddress(txHash)}`);
    } catch (error) {
      setStatusMessage(`Submission failed: ${String(error)}`);
    } finally {
      setIsSubmitting(false);
    }
  };

  const checkStatus = async () => {
    setStatusResult(null);
    if (!statusIntentId || !isHex(statusIntentId)) {
      setStatusResult('Provide a valid intent ID.');
      return;
    }
    try {
      setIsCheckingStatus(true);
      const response = await fetch(`/api/gateway/intents/${statusIntentId}`);
      if (!response.ok) {
        setStatusResult(`Status query failed: ${await extractGatewayError(response)}`);
        return;
      }
      const payload = await response.json();
      setStatusResult(`Status: ${payload.status || 'unknown'} · Batch: ${payload.batch_id ?? '—'}`);
    } catch (error) {
      setStatusResult(`Status query failed: ${String(error)}`);
    } finally {
      setIsCheckingStatus(false);
    }
  };

  const cancelIntent = async () => {
    setStatusResult(null);
    if (!walletAddress) {
      setStatusResult('Connect a wallet before canceling.');
      return;
    }
    if (!statusIntentId || !isHex(statusIntentId)) {
      setStatusResult('Provide a valid intent ID.');
      return;
    }

    try {
      setIsCanceling(true);
      const activeWallet = await ensureWalletSession(wallet);
      const tx = await activeWallet.execute(
        [buildCancelIntentCall(statusIntentId)],
        { feeMode: 'sponsored' },
      );
      await tx.wait();
      const txHash = tx.hash;

      const response = await fetch(`/api/gateway/intents/${statusIntentId}/onchain`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          action: 'CANCELED',
          user_address: activeWallet.address,
          tx_hash: txHash,
        }),
      });
      if (!response.ok) {
        setStatusResult(
          `Cancel committed on-chain. Gateway reconciliation is pending via the observer. Tx ${truncateAddress(txHash)}`
        );
        return;
      }

      setStatusResult(`Intent canceled on-chain. Tx ${truncateAddress(txHash)}`);
    } catch (error) {
      setStatusResult(`Cancel failed: ${String(error)}`);
    } finally {
      setIsCanceling(false);
    }
  };

  return (
    <motion.section
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.5 }}
      className="glass-card glass-card-hover gradient-border relative overflow-hidden"
    >
      <div className="relative z-10 p-6 md:p-8">
        <div className="flex items-center justify-between gap-4">
          <div>
            <p className="text-sm uppercase tracking-[0.2em] text-text-muted">Intent Composer</p>
            <h2 className="text-2xl font-semibold md:text-3xl">Execute with Privacy</h2>
          </div>
          <div className="rounded-full border border-white/10 bg-white/5 px-3 py-1 text-sm">
            {walletAddress ? `Wallet ${truncateAddress(walletAddress)}` : 'Connect Wallet'}
          </div>
        </div>

        <div className="mt-6 grid gap-4">
          <div className="flex items-center gap-3">
            <div className="flex-1 rounded-xl border border-white/10 bg-white/5 p-4">
              <div className="flex items-center justify-between">
                <p className="text-xs text-text-muted">From</p>
                {assetInBalance !== null && (
                  <p className="text-xs text-text-muted">Balance: {assetInBalance} {draft.assetIn ? tokenSymbol(draft.assetIn) : ''}</p>
                )}
              </div>
              <div className="mt-2 flex items-center justify-between gap-2">
                <div className="flex min-w-0 flex-1 flex-col">
                  {draft.assetIn && (
                    <span className="text-sm font-semibold text-text-primary">{tokenSymbol(draft.assetIn)}</span>
                  )}
                  <input
                    value={draft.assetIn}
                    onChange={(event) => setDraft({ assetIn: event.target.value })}
                    placeholder="0x token address"
                    className="w-full bg-transparent text-xs text-text-muted outline-none"
                  />
                </div>
                <button
                  type="button"
                  onClick={() => setDraft({ assetIn: draft.assetOut, assetOut: draft.assetIn })}
                  className="shrink-0 text-xs text-text-secondary hover:text-text-primary"
                >
                  Switch
                </button>
              </div>
            </div>
            <div className="text-2xl text-text-muted">→</div>
            <div className="flex-1 rounded-xl border border-white/10 bg-white/5 p-4">
              <p className="text-xs text-text-muted">To</p>
              <div className="mt-2 flex min-w-0 flex-col">
                {draft.assetOut && (
                  <span className="text-sm font-semibold text-text-primary">{tokenSymbol(draft.assetOut)}</span>
                )}
                <input
                  value={draft.assetOut}
                  onChange={(event) => setDraft({ assetOut: event.target.value })}
                  placeholder="0x token address"
                  className="w-full bg-transparent text-xs text-text-muted outline-none"
                />
              </div>
            </div>
          </div>

          <div className="grid gap-3 md:grid-cols-2">
            <div className="rounded-xl border border-white/10 bg-white/5 p-4">
              <p className="text-xs text-text-muted">Amount (base units)</p>
              <div className="mt-2 flex items-end justify-between gap-3">
                <input
                  type="text"
                  value={draft.amount}
                  onChange={(event) => setDraft({ amount: event.target.value })}
                  className="w-full bg-transparent text-2xl font-semibold outline-none"
                  placeholder="0"
                />
                <span className="text-xs text-text-secondary">{draft.assetIn ? tokenSymbol(draft.assetIn) : '—'}</span>
              </div>
              <p className="mt-1 text-xs text-text-muted">Authorization comes from the connected wallet.</p>
            </div>
            <div className="rounded-xl border border-white/10 bg-white/5 p-4">
              <p className="text-xs text-text-muted">Deadline</p>
              <div className="mt-2 flex items-end justify-between gap-3">
                <input
                  type="number"
                  min="1"
                  max={MAX_INTENT_DEADLINE_MINUTES}
                  step="1"
                  value={draft.deadlineMinutes}
                  onChange={(event) => setDraft({ deadlineMinutes: normalizeDeadlineMinutesInput(event.target.value) })}
                  className="w-full bg-transparent text-2xl font-semibold outline-none"
                  placeholder="0"
                />
                <span className="text-xs text-text-secondary">minutes</span>
              </div>
              <p className="mt-1 text-xs text-text-muted">
                Stored at gateway, committed with your wallet. Max {MAX_INTENT_DEADLINE_MINUTES} minute{MAX_INTENT_DEADLINE_MINUTES === 1 ? '' : 's'}.
              </p>
            </div>
          </div>

          <div className="grid gap-3 md:grid-cols-2">
            <div className="rounded-xl border border-white/10 bg-white/5 p-4">
              <p className="text-xs text-text-muted">Connected Wallet</p>
              <p className="mt-2 break-all text-sm text-text-secondary">{walletAddress || 'No Starknet wallet connected'}</p>
            </div>
            <div className="rounded-xl border border-white/10 bg-white/5 p-4">
              <p className="text-xs text-text-muted">Submission Mode</p>
              <p className="mt-2 text-sm text-text-secondary">Gateway stores encrypted payload, wallet sends the on-chain commit.</p>
            </div>
          </div>

          {(intentHash || intentId) ? (
            <div className="grid gap-3 md:grid-cols-2">
              <div className="rounded-xl border border-white/10 bg-white/5 p-4">
                <p className="text-xs text-text-muted">Intent Hash</p>
                <p className="mt-2 break-all text-xs text-text-secondary">{intentHash || '—'}</p>
              </div>
              <div className="rounded-xl border border-white/10 bg-white/5 p-4">
                <p className="text-xs text-text-muted">Intent ID</p>
                <p className="mt-2 break-all text-xs text-text-secondary">{intentId || '—'}</p>
              </div>
            </div>
          ) : null}

          <div className="grid gap-3 md:grid-cols-2">
            <label className="rounded-xl border border-white/10 bg-white/5 p-4">
              <span className="text-xs text-text-muted">Intent ID (status/cancel)</span>
              <input
                value={statusIntentId}
                onChange={(event) => setStatusIntentId(event.target.value)}
                placeholder="0x..."
                className="mt-2 w-full bg-transparent text-sm outline-none"
              />
            </label>
            <div className="rounded-xl border border-white/10 bg-white/5 p-4">
              <p className="text-xs text-text-muted">Status Actions</p>
              <div className="mt-2 flex flex-wrap gap-2">
                <button
                  type="button"
                  onClick={checkStatus}
                  disabled={isCheckingStatus}
                  className="rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-xs text-text-secondary disabled:opacity-50"
                >
                  {isCheckingStatus ? 'Checking…' : 'Check Status'}
                </button>
                <button
                  type="button"
                  onClick={cancelIntent}
                  disabled={isCanceling}
                  className="rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-xs text-text-secondary disabled:opacity-50"
                >
                  {isCanceling ? 'Canceling…' : 'Cancel On-Chain'}
                </button>
              </div>
              {statusResult ? <p className="mt-2 text-xs text-text-muted">{statusResult}</p> : null}
            </div>
          </div>

          <div className="rounded-2xl border border-white/10 bg-white/3 p-5">
            <div className="flex items-center justify-between">
              <p className="text-sm text-text-secondary">Constraints</p>
              <button
                type="button"
                onClick={() => setShowAdvanced((prev) => !prev)}
                className="text-xs text-text-muted"
              >
                {showAdvanced ? 'Advanced ▴' : 'Advanced ▾'}
              </button>
            </div>

            {showAdvanced ? (
              <div className="mt-4 space-y-4">
                <div>
                  <div className="flex items-center justify-between text-xs text-text-muted">
                    <span>Min Output</span>
                    <span className="text-text-secondary">
                      {formatIntegerString(draft.minOutput)} {draft.assetOut ? tokenSymbol(draft.assetOut) : ''}
                    </span>
                  </div>
                  <input
                    type="range"
                    min={0}
                    max={1000}
                    step={1}
                    value={minOutputSliderValue}
                    onChange={(event) => {
                      const sliderValue = BigInt(event.target.value);
                      const nextValue = sliderValue === 0n
                        ? 0n
                        : clampBigint((minOutputCap * sliderValue) / 1000n, 1n, minOutputCap);
                      setDraft({ minOutput: nextValue.toString() });
                    }}
                    className="mt-2 w-full"
                  />
                  <p className="mt-2 text-[11px] text-text-muted">
                    Uses a relative scale so large base-unit numbers do not jump to the end of the rail.
                  </p>
                </div>
                <div>
                  <div className="flex items-center justify-between text-xs text-text-muted">
                    <span>Max Fee</span>
                    <span className="text-text-secondary">{maxFeePercent.toFixed(2)}%</span>
                  </div>
                  <input
                    type="range"
                    min={0}
                    max={1}
                    step={0.01}
                    value={maxFeePercent}
                    onChange={(event) => {
                      const value = Number(event.target.value);
                      setDraft({ maxFeeBps: Math.round(value * 100) });
                    }}
                    className="mt-2 w-full"
                  />
                </div>
              </div>
            ) : null}
          </div>

          <div className="space-y-2">
            <p className="text-sm text-text-secondary">Privacy Mode</p>
            <div className="grid gap-2 md:grid-cols-3">
              {privacyOptions.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  onClick={() => {
                    setPrivacy(option.id);
                    setDraft({ privacyMode: option.id as typeof draft.privacyMode });
                  }}
                  className={clsx(
                    'rounded-xl border px-3 py-3 text-left transition',
                    privacy === option.id ? 'border-white/30 bg-white/10' : 'border-white/10 bg-white/5'
                  )}
                >
                  <div className="flex items-center gap-2">
                    <span className={clsx('text-lg', option.color)}>{option.icon}</span>
                    <div>
                      <p className="text-sm font-medium">{option.label}</p>
                      <p className="text-xs text-text-muted">{option.description}</p>
                    </div>
                  </div>
                </button>
              ))}
            </div>
          </div>

          <div className="flex flex-col gap-3 md:flex-row md:items-center md:justify-between">
            <div>
              <p className="text-xs text-text-muted">Estimated Execution</p>
              <p className="text-sm">Wallet signature + on-chain commit + batch assignment</p>
            </div>
            <div>
              <p className="text-xs text-text-muted">Total Cost</p>
              <p className="text-sm">Wallet gas + solver fee ceiling {maxFeePercent.toFixed(2)}%</p>
            </div>
            <div className="flex flex-col gap-2">
              <button
                type="button"
                onClick={prepareIntent}
                className="rounded-xl border border-white/10 bg-white/5 px-6 py-3 text-sm font-semibold text-text-secondary"
              >
                Prepare Intent
              </button>
              <button
                type="button"
                onClick={onSubmit}
                disabled={isSubmitting}
                className="rounded-xl bg-accent-primary px-6 py-3 text-sm font-semibold text-white disabled:cursor-not-allowed disabled:opacity-50"
              >
                {isSubmitting ? 'Submitting…' : 'Submit With Wallet'}
              </button>
            </div>
          </div>

          {statusMessage ? (
            <div className="rounded-xl border border-white/10 bg-white/5 px-4 py-3 text-xs text-text-secondary">
              {statusMessage}
            </div>
          ) : null}
        </div>
      </div>
    </motion.section>
  );
}

async function ensureWalletSession(
  wallet: ReturnType<typeof useIntentStore.getState>['wallet'],
) {
  if (wallet) {
    return wallet;
  }
  throw new Error('Wallet not connected. Connect your wallet from the /connect page.');
}

async function extractGatewayError(response: Response): Promise<string> {
  const body = await response.text();
  if (!body) {
    return `HTTP ${response.status}`;
  }

  try {
    const parsed = JSON.parse(body) as { error?: string };
    return parsed.error ?? body;
  } catch {
    return body;
  }
}

function parseBigint(value: string): bigint {
  const normalized = value.trim();
  if (!/^\d+$/.test(normalized)) {
    throw new Error('Amounts must be provided as whole numbers');
  }
  return BigInt(normalized);
}

function parseOptionalBigint(value: string): bigint {
  const normalized = value.trim();
  if (!/^\d+$/.test(normalized)) {
    return 0n;
  }
  return BigInt(normalized);
}

function isHex(value: string): boolean {
  const normalized = value.startsWith('0x') ? value.slice(2) : value;
  return normalized.length > 0 && /^[0-9a-fA-F]+$/.test(normalized);
}

function isZeroAddress(value: string): boolean {
  const normalized = value.startsWith('0x') ? value.slice(2) : value;
  return normalized.length === 0 || /^0+$/.test(normalized);
}

function normalizeHex(value: string): string {
  const normalized = value.trim();
  if (!normalized) {
    return normalized;
  }
  if (normalized.startsWith('0x') || normalized.startsWith('0X')) {
    return `0x${normalized.slice(2).toLowerCase()}`;
  }
  if (/^\d+$/.test(normalized)) {
    return `0x${BigInt(normalized).toString(16)}`;
  }
  return `0x${normalized.toLowerCase()}`;
}

function validateDeadlineMinutes(value: number): string | null {
  if (!Number.isFinite(value) || value < 1) {
    return 'Deadline must be at least 1 minute.';
  }
  if (value > MAX_INTENT_DEADLINE_MINUTES) {
    return `Deadline exceeds the gateway limit of ${MAX_INTENT_DEADLINE_MINUTES} minute${MAX_INTENT_DEADLINE_MINUTES === 1 ? '' : 's'}.`;
  }
  return null;
}

function resolveMinOutputCap(amount: bigint, currentMinOutput: bigint): bigint {
  const base = maxBigint(amount, currentMinOutput, 1n);
  const padded = base + (base / 5n) + 1n;
  return maxBigint(padded, 100n);
}

function clampBigint(value: bigint, min: bigint, max: bigint): bigint {
  if (value < min) {
    return min;
  }
  if (value > max) {
    return max;
  }
  return value;
}

function maxBigint(...values: bigint[]): bigint {
  return values.reduce((current, value) => (value > current ? value : current), values[0] ?? 0n);
}

function formatIntegerString(value: string): string {
  const normalized = value.trim();
  if (!/^\d+$/.test(normalized)) {
    return '0';
  }
  return normalized.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

function normalizeDeadlineMinutesInput(value: string): number {
  const parsed = Math.floor(Number(value));
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return 0;
  }
  return Math.min(parsed, MAX_INTENT_DEADLINE_MINUTES);
}
