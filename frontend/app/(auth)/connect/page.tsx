import Link from 'next/link';
import dynamic from 'next/dynamic';

const WalletConnect = dynamic(() => import('@/components/WalletConnect'), { ssr: false });

export default function ConnectPage() {
  return (
    <main className="flex min-h-screen items-center justify-center px-4">
      <div className="w-full max-w-xl space-y-6">
        <header className="text-center">
          <p className="text-xs uppercase tracking-[0.3em] text-text-muted">Connect</p>
          <h1 className="mt-2 text-3xl font-semibold">Connect Wallet</h1>
          <p className="mt-2 text-sm text-text-secondary">
            Continue with Email, Argent, or Braavos to access the desk routes.
          </p>
        </header>
        <WalletConnect />
        <div className="text-center">
          <Link href="/" className="text-sm text-accent-primary">
            Back to landing
          </Link>
        </div>
      </div>
    </main>
  );
}
