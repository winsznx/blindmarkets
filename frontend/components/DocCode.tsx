'use client';

import { useState } from 'react';

export function DocCode({
  filename,
  language,
  children,
}: {
  filename?: string;
  language?: string;
  children: string;
}) {
  const [copied, setCopied] = useState(false);

  function copy() {
    navigator.clipboard.writeText(children.trim()).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    });
  }

  const hasHeader = filename || language;

  return (
    <div className="not-prose my-4 rounded-xl overflow-hidden" style={{ border: '1px solid rgba(255,255,255,0.06)' }}>
      {hasHeader && (
        <div
          className="flex items-center justify-between px-4 py-2 text-xs"
          style={{ background: 'rgba(255,255,255,0.03)', borderBottom: '1px solid rgba(255,255,255,0.06)' }}
        >
          <div className="flex items-center gap-2">
            {filename && (
              <span className="font-mono" style={{ color: 'rgba(255,255,255,0.5)' }}>{filename}</span>
            )}
            {language && !filename && (
              <span className="font-mono" style={{ color: 'rgba(255,255,255,0.35)' }}>{language}</span>
            )}
          </div>
          <button
            onClick={copy}
            className="transition-colors px-2 py-0.5 rounded text-xs"
            style={{ color: copied ? 'var(--status-success)' : 'rgba(255,255,255,0.35)' }}
          >
            {copied ? 'Copied' : 'Copy'}
          </button>
        </div>
      )}
      <pre
        className="overflow-x-auto px-5 py-4 text-sm leading-relaxed"
        style={{ background: 'rgba(0,0,0,0.4)', margin: 0 }}
      >
        <code style={{ color: 'var(--text-primary)', fontFamily: "'JetBrains Mono', 'Fira Code', monospace" }}>
          {children.trim()}
        </code>
      </pre>
    </div>
  );
}
