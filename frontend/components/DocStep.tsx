export function DocStep({
  n,
  title,
  children,
}: {
  n: number;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex gap-4 mb-8">
      <div
        className="shrink-0 w-8 h-8 rounded-full flex items-center justify-center text-sm font-semibold"
        style={{
          background: 'color-mix(in srgb, var(--accent) 8%, transparent)',
          border: '1px solid color-mix(in srgb, var(--accent) 20%, transparent)',
          color: 'var(--accent)',
        }}
      >
        {n}
      </div>
      <div className="flex-1 min-w-0">
        <h3 className="text-white font-semibold mb-1.5 mt-0">{title}</h3>
        <div className="text-white/60 text-sm leading-relaxed space-y-2">{children}</div>
      </div>
    </div>
  );
}

export function DocSteps({ children }: { children: React.ReactNode }) {
  return (
    <div className="not-prose mt-6 border-l-2 border-white/5 pl-0">
      {children}
    </div>
  );
}
