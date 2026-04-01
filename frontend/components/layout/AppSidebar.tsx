'use client';

import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { BarChart2, Clock, Crosshair, ShieldAlert, Zap } from 'lucide-react';
import { useUIStore } from '@/state/useUIStore';
import BrandMark from '@/components/BrandMark';
import NetworkBadge from '@/components/wallet/NetworkBadge';
import WalletStatus from '@/components/wallet/WalletStatus';

type NavLink = {
  href: string;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
  requiresSolverMode?: boolean;
};

const NAV_LINKS: NavLink[] = [
  { href: '/desk', label: 'Desk', icon: Crosshair },
  { href: '/history', label: 'History', icon: Clock },
  { href: '/analytics', label: 'Analytics', icon: BarChart2 },
  { href: '/risk', label: 'Risk', icon: ShieldAlert },
  { href: '/solver', label: 'Solver', icon: Zap, requiresSolverMode: true },
];

export default function AppSidebar() {
  const pathname = usePathname();
  const sidebarOpen = useUIStore((s) => s.sidebarOpen);
  const solverModeEnabled = process.env.NEXT_PUBLIC_SOLVER_MODE === 'true';

  if (!sidebarOpen) return null;

  return (
    <aside className="fixed inset-y-0 left-0 z-40 hidden w-16 flex-col border-r border-white/10 bg-bg-base/90 px-2 py-4 backdrop-blur md:flex xl:w-[220px] xl:px-4 xl:py-6">
      <Link href="/desk" className="flex justify-center xl:justify-start">
        <BrandMark subtitle="Execution desk" showText={false} className="xl:hidden" />
        <BrandMark subtitle="Execution desk" className="hidden xl:flex" />
      </Link>

      <nav className="mt-6 space-y-2">
        {NAV_LINKS.filter((link) => !link.requiresSolverMode || solverModeEnabled).map((link) => {
          const isActive = pathname === link.href || pathname.startsWith(`${link.href}/`);
          const Icon = link.icon;
          return (
            <Link
              key={link.href}
              href={link.href}
              className={`flex items-center justify-center gap-3 rounded-lg border px-2 py-2 text-text-secondary transition hover:text-text-primary xl:justify-start xl:px-3 ${
                isActive ? 'border-white/20 bg-white/10 text-text-primary' : 'border-white/10 bg-white/5'
              }`}
            >
              <Icon className="h-4 w-4 shrink-0" />
              <span className="hidden text-sm xl:inline">{link.label}</span>
            </Link>
          );
        })}
      </nav>

      <div className="mt-auto rounded-lg border border-white/10 bg-white/5 p-2 text-center xl:p-3 xl:text-left">
        <p className="hidden text-[11px] uppercase tracking-[0.2em] text-text-muted xl:block">Wallet</p>
        <div className="mt-1 hidden xl:block">
          <WalletStatus compact />
        </div>
        <div className="mt-2 hidden xl:block">
          <NetworkBadge />
        </div>
      </div>
    </aside>
  );
}
