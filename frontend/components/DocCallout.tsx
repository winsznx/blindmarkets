type CalloutType = 'note' | 'tip' | 'warning' | 'danger';

const configs: Record<CalloutType, { label: string; icon: string; bg: string; border: string; iconColor: string; labelColor: string }> = {
  note: {
    label: 'Note',
    icon: 'ℹ',
    bg: 'color-mix(in srgb, var(--accent) 5%, transparent)',
    border: 'color-mix(in srgb, var(--accent) 20%, transparent)',
    iconColor: 'var(--accent)',
    labelColor: 'var(--accent)',
  },
  tip: {
    label: 'Tip',
    icon: '✦',
    bg: 'color-mix(in srgb, var(--status-success) 5%, transparent)',
    border: 'color-mix(in srgb, var(--status-success) 20%, transparent)',
    iconColor: 'var(--status-success)',
    labelColor: 'var(--status-success)',
  },
  warning: {
    label: 'Warning',
    icon: '⚠',
    bg: 'color-mix(in srgb, var(--status-pending) 5%, transparent)',
    border: 'color-mix(in srgb, var(--status-pending) 20%, transparent)',
    iconColor: 'var(--status-pending)',
    labelColor: 'var(--status-pending)',
  },
  danger: {
    label: 'Danger',
    icon: '✕',
    bg: 'color-mix(in srgb, var(--status-error) 5%, transparent)',
    border: 'color-mix(in srgb, var(--status-error) 20%, transparent)',
    iconColor: 'var(--status-error)',
    labelColor: 'var(--status-error)',
  },
};

export function DocCallout({
  type = 'note',
  title,
  children,
}: {
  type?: CalloutType;
  title?: string;
  children: React.ReactNode;
}) {
  const c = configs[type];
  return (
    <div
      className="not-prose my-5 rounded-xl px-4 py-3.5 text-sm leading-relaxed"
      style={{ background: c.bg, border: `1px solid ${c.border}` }}
    >
      <div className="flex gap-2.5 items-start">
        <span className="shrink-0 mt-0.5 text-base leading-none" style={{ color: c.iconColor }}>
          {c.icon}
        </span>
        <div>
          {title ? (
            <p className="font-semibold mb-1" style={{ color: c.labelColor }}>{title}</p>
          ) : (
            <span className="font-semibold mr-1.5" style={{ color: c.labelColor }}>{c.label}</span>
          )}
          <span className="text-white/60">{children}</span>
        </div>
      </div>
    </div>
  );
}
