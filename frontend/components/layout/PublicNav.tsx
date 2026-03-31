'use client';

import Link from 'next/link';
import { Menu, X } from 'lucide-react';
import { useEffect, useState } from 'react';
import BrandMark from '@/components/BrandMark';

const PUBLIC_LINKS = [
  { href: '/', label: 'Features' },
  { href: '/docs/how-it-works', label: 'How It Works' },
  { href: '/docs/traders', label: 'Risk' },
  { href: '/docs', label: 'Docs' },
];

export default function PublicNav() {
  const [scrolled, setScrolled] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);

  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 10);
    onScroll();
    window.addEventListener('scroll', onScroll, { passive: true });
    return () => window.removeEventListener('scroll', onScroll);
  }, []);

  return (
    <>
      <nav className={`sticky top-4 z-40 mx-auto flex w-full max-w-7xl items-center justify-between gap-4 rounded-2xl px-4 py-3 sm:px-6 ${scrolled ? 'glass-card' : 'border border-transparent'}`}>
        <Link href="/">
          <BrandMark subtitle="Private BTC intents" />
        </Link>
        <div className="hidden items-center gap-5 text-xs uppercase tracking-[0.2em] text-text-muted md:flex">
          {PUBLIC_LINKS.map((link) => (
            <Link key={link.href} href={link.href}>
              {link.label}
            </Link>
          ))}
        </div>
        <div className="hidden items-center gap-2 md:flex">
          <Link href="/desk" className="rounded-lg bg-accent-primary px-4 py-2 text-xs font-semibold text-white">
            Open Desk
          </Link>
          <Link href="/connect" className="rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-xs font-semibold text-text-secondary">
            Connect
          </Link>
        </div>
        <button
          type="button"
          onClick={() => setMenuOpen((prev) => !prev)}
          className="rounded-lg border border-white/10 bg-white/5 p-2 text-text-secondary md:hidden"
          aria-label="Toggle menu"
        >
          {menuOpen ? <X className="h-4 w-4" /> : <Menu className="h-4 w-4" />}
        </button>
      </nav>

      {menuOpen ? (
        <div className="fixed inset-0 z-30 flex flex-col gap-6 bg-bg-overlay px-6 py-24 md:hidden">
          {PUBLIC_LINKS.map((link) => (
            <Link
              key={link.href}
              href={link.href}
              className="text-xl font-semibold text-text-primary"
              onClick={() => setMenuOpen(false)}
            >
              {link.label}
            </Link>
          ))}
          <Link href="/desk" className="rounded-lg bg-accent-primary px-4 py-3 text-center text-sm font-semibold text-white" onClick={() => setMenuOpen(false)}>
            Open Desk
          </Link>
          <Link href="/connect" className="rounded-lg border border-white/10 bg-white/5 px-4 py-3 text-center text-sm font-semibold text-text-secondary" onClick={() => setMenuOpen(false)}>
            Connect Wallet
          </Link>
        </div>
      ) : null}
    </>
  );
}
