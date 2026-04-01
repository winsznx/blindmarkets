'use client';

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { useIntentStore } from '../state/useIntentStore';
import {
  connectWithEmail,
  connectWithArgent,
  connectWithBraavos,
  truncateAddress,
  type WalletProviderKey,
} from '../lib/starkzap-wallet';

const PROVIDERS: Array<{
  key: WalletProviderKey;
  label: string;
  connect: () => Promise<{ address: string }>;
}> = [
  { key: 'email', label: 'Continue with Email', connect: connectWithEmail },
  { key: 'argent', label: 'Connect Argent', connect: connectWithArgent },
  { key: 'braavos', label: 'Connect Braavos', connect: connectWithBraavos },
];

export default function WalletConnect() {
  const router = useRouter();
  const { wallet, walletAddress, walletProviderKey, setWalletSession, clearWalletSession } = useIntentStore();
  const [status, setStatus] = useState<string | null>(null);
  const [connecting, setConnecting] = useState<WalletProviderKey | null>(null);

  useEffect(() => {
    if (wallet || !walletProviderKey || !walletAddress) return;

    const reconnectors: Partial<Record<WalletProviderKey, () => Promise<{ address: string }>>> = {
      argent: connectWithArgent,
      braavos: connectWithBraavos,
    };
    const reconnect = reconnectors[walletProviderKey];
    if (!reconnect) return;

    reconnect()
      .then((w) => setWalletSession(w as Parameters<typeof setWalletSession>[0], w.address, walletProviderKey))
      .catch(() => clearWalletSession());
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onConnect = async (providerKey: WalletProviderKey, connect: () => Promise<{ address: string }>) => {
    setStatus(null);
    setConnecting(providerKey);
    try {
      const w = await connect();
      setWalletSession(w as Parameters<typeof setWalletSession>[0], w.address, providerKey);
      document.cookie = `blindmarkets-wallet=${w.address}; path=/; SameSite=Lax`;
      const next = new URLSearchParams(window.location.search).get('next') ?? '/desk';
      router.push(next);
    } catch (error) {
      setStatus(`Connection failed: ${String(error)}`);
    } finally {
      setConnecting(null);
    }
  };

  return (
    <div className="glass-card glass-card-hover p-4">
      <div className="flex items-center justify-between">
        <p className="text-xs uppercase tracking-[0.2em] text-text-muted">Wallets</p>
        {walletAddress ? (
          <button
            type="button"
            onClick={() => {
              clearWalletSession();
              document.cookie =
                'blindmarkets-wallet=; expires=Thu, 01 Jan 1970 00:00:00 UTC; path=/';
              setStatus('Wallet disconnected.');
            }}
            className="rounded-lg border border-white/10 bg-white/5 px-3 py-1 text-[11px] text-text-secondary"
          >
            Disconnect
          </button>
        ) : null}
      </div>

      <div className="mt-3 grid gap-2">
        {PROVIDERS.map(({ key, label, connect }) => (
          <button
            key={key}
            type="button"
            onClick={() => onConnect(key, connect)}
            disabled={connecting !== null}
            className="rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-sm text-text-secondary disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {connecting === key ? `Connecting…` : label}
          </button>
        ))}
      </div>

      <div className="mt-3 rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-xs text-text-secondary">
        {walletAddress ? truncateAddress(walletAddress) : 'Not connected'}
      </div>

      {wallet && (
        <div className="mt-1 rounded-lg border border-white/10 bg-white/5 px-3 py-2 text-xs text-text-secondary break-all">
          {walletAddress}
        </div>
      )}

      {status ? <p className="mt-2 text-xs text-text-muted">{status}</p> : null}
    </div>
  );
}
