import type { RefObject } from "react";

interface Props {
  lines: string[];
  scrollRef: RefObject<HTMLDivElement | null>;
}

export function LogViewer({ lines, scrollRef }: Props) {
  return (
    <div
      ref={scrollRef}
      className="flex-1 overflow-y-auto bg-jupiter-bg rounded border border-jupiter-orange/25 p-2 font-mono text-[11px] leading-5 min-h-0"
    >
      {lines.length === 0 ? (
        <p className="text-jupiter-dim italic">No logs yet.</p>
      ) : (
        lines.map((line, i) => (
          <div key={i} className="whitespace-pre-wrap break-all text-white">
            {line}
          </div>
        ))
      )}
    </div>
  );
}
