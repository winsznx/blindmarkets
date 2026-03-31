'use client';

import { Area, AreaChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts';
import { useEffect, useState } from 'react';

type BatchListItem = {
  batch_id: string;
  close_time: number;
  intent_count: number;
  status?: string;
};

type BatchListResponse = {
  batches: BatchListItem[];
  limit: number;
  offset: number;
};

export default function ExecutionChart() {
  const [data, setData] = useState<{ name: string; value: number }[]>([]);
  const [isLoading, setIsLoading] = useState(false);

  useEffect(() => {
    let isMounted = true;
    const run = async () => {
      setIsLoading(true);
      try {
        const response = await fetch('/api/gateway/batches?limit=10');
        if (!response.ok) {
          return;
        }
        const payload: BatchListResponse = await response.json();
        const chartData = (payload.batches ?? [])
          .slice()
          .reverse()
          .map((batch) => ({
            name: `#${batch.batch_id}`,
            value: batch.intent_count
          }));
        if (isMounted) {
          setData(chartData);
        }
      } finally {
        if (isMounted) {
          setIsLoading(false);
        }
      }
    };
    run();
    return () => {
      isMounted = false;
    };
  }, []);

  return (
    <div className="glass-card glass-card-hover p-6">
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold">Batch Throughput</h3>
        <span className="text-xs text-text-muted">Intents per batch</span>
      </div>
      <div className="mt-4 h-48">
        {isLoading && (
          <div className="flex h-full items-center justify-center text-sm text-text-secondary">
            Loading batch data...
          </div>
        )}
        {!isLoading && data.length === 0 && (
          <div className="flex h-full items-center justify-center text-sm text-text-secondary">
            No batch history yet.
          </div>
        )}
        {!isLoading && data.length > 0 && (
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={data}>
              <XAxis dataKey="name" stroke="var(--text-muted)" tickLine={false} axisLine={false} />
              <YAxis hide />
              <Tooltip content={<TooltipContent />} />
              <Area type="monotone" dataKey="value" stroke="var(--accent)" fill="var(--accent-subtle)" />
            </AreaChart>
          </ResponsiveContainer>
        )}
      </div>
    </div>
  );
}

function TooltipContent({ active, payload }: { active?: boolean; payload?: Array<{ value: number }> }) {
  if (!active || !payload || payload.length === 0) {
    return null;
  }
  return (
    <div className="rounded-lg border border-white/10 bg-bg-secondary/90 px-3 py-2 text-xs text-text-secondary shadow-glass">
      <span>{payload[0]?.value ?? 0} intents</span>
    </div>
  );
}
