'use client';

import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { LogOut } from 'lucide-react';
import { useIntentStore } from '@/state/useIntentStore';
import AddressDisplay from '@/components/wallet/AddressDisplay';

type WalletStatusProps = {
  compact?: boolean;
};

export default function WalletStatus({ compact = false }: WalletStatusProps) {
  const router = useRouter();
  const walletAddress = useIntentStore((state) => state.walletAddress);
  const clearWalletSession = useIntentStore((state) => state.clearWalletSession);

  if (!walletAddress) {
    return (
      <Link
        href="/connect"
        className="rounded-lg border border-white/10 bg-white/5 px-3 py-1.5 text-xs text-text-secondary"
      >
        Connect
      </Link>
    );
  }

  const handleDisconnect = () => {
    clearWalletSession();
    document.cookie = 'blindmarkets-wallet=; expires=Thu, 01 Jan 1970 00:00:00 UTC; path=/';
    router.push('/connect');
  };

  return (
    <div className="flex items-center gap-1 rounded-lg border border-white/10 bg-white/5 px-2 py-1.5">
      <AddressDisplay address={walletAddress} truncate={compact} />
      <button
        type="button"
        onClick={handleDisconnect}
        className="rounded border border-white/10 bg-white/5 p-1 hover:text-text-primary"
        aria-label="Disconnect wallet"
      >
        <LogOut className="h-3 w-3" />
      </button>
    </div>
  );
}
