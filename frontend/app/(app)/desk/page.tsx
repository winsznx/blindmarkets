import Link from 'next/link';
import dynamic from 'next/dynamic';
import BatchTimeline from '@/components/BatchTimeline';
import SolverFillPreview from '@/components/SolverFillPreview';
import RiskDisclosurePanel from '@/components/RiskDisclosurePanel';
import AuditLogView from '@/components/AuditLogView';

// starkzap is browser-only (wallet APIs). Disable SSR to prevent the prerender
// from executing starkzap's module graph against the server's webpack bundle.
const IntentComposer = dynamic(() => import('@/components/IntentComposer'), { ssr: false });
const WalletConnect = dynamic(() => import('@/components/WalletConnect'), { ssr: false });

export default function DashboardPage() {
  return (
    <div className="space-y-10">
      <header className="flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
        <div>
          <p className="text-xs uppercase tracking-[0.3em] text-text-muted">Desk</p>
          <h1 className="text-3xl font-semibold">Execution Desk</h1>
        </div>
        <Link href="/history" className="rounded-xl bg-accent-primary px-4 py-2 text-sm font-semibold text-white">
          View History
        </Link>
      </header>

      <section className="mt-10 grid gap-6 lg:grid-cols-[2fr_1fr]">
        <IntentComposer />
        <div id="wallets" className="space-y-6">
          <BatchTimeline />
          <WalletConnect />
        </div>
      </section>

      <section id="intent-history" className="mt-10 grid gap-6 lg:grid-cols-3">
        <SolverFillPreview />
        <RiskDisclosurePanel />
        <AuditLogView />
      </section>
    </div>
  );
}
