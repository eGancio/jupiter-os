import { useState } from "react";

interface Props {
  thinking: string;
  streaming?: boolean;
}

export function ThinkingBlock({ thinking, streaming }: Props) {
  const [expanded, setExpanded] = useState(false);

  // Count lines for summary
  const lines = thinking.split("\n").filter((l) => l.trim().length > 0);
  const charCount = thinking.length;

  // Preview: first meaningful line, truncated
  const firstLine = lines[0] || "";
  const preview = firstLine.length > 100 ? firstLine.slice(0, 97) + "..." : firstLine;

  return (
    <div className="mb-2">
      <button
        onClick={() => setExpanded(!expanded)}
        className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-jupiter-surface/60 border border-jupiter-blue/20 hover:border-jupiter-blue/40 transition-colors w-full text-left group"
      >
        {/* Thinking icon */}
        <span className="flex-shrink-0">
          {streaming ? (
            <span className="flex gap-0.5">
              <span className="w-1 h-1 rounded-full bg-jupiter-blue animate-bounce" style={{ animationDelay: "0ms" }} />
              <span className="w-1 h-1 rounded-full bg-jupiter-blue animate-bounce" style={{ animationDelay: "100ms" }} />
              <span className="w-1 h-1 rounded-full bg-jupiter-blue animate-bounce" style={{ animationDelay: "200ms" }} />
            </span>
          ) : (
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="text-jupiter-blue">
              <path d="M12 2a8 8 0 0 1 8 8c0 3-1.5 5-3.5 6.5L16 22H8l-.5-5.5C5.5 15 4 13 4 10a8 8 0 0 1 8-8z" />
              <path d="M9 22h6" />
            </svg>
          )}
        </span>

        <span className="text-[0.85em] font-medium text-jupiter-blue">
          {streaming ? "Thinking..." : "Thinking"}
        </span>

        {!streaming && (
          <span className="text-[0.8em] text-jupiter-dim ml-1">
            {charCount > 1000 ? `${Math.round(charCount / 1000)}k chars` : `${charCount} chars`}
          </span>
        )}

        {/* Preview when collapsed */}
        {!expanded && !streaming && preview && (
          <span className="text-[0.8em] text-jupiter-muted truncate ml-1 flex-1 min-w-0">
            {preview}
          </span>
        )}

        <span className="ml-auto text-jupiter-dim text-[0.8em] flex-shrink-0 group-hover:text-jupiter-blue transition-colors">
          {expanded ? "▾" : "▸"}
        </span>
      </button>

      {/* Expanded thinking content */}
      {expanded && (
        <div className="mt-1 rounded-md bg-jupiter-surface/40 border border-jupiter-blue/15 overflow-hidden">
          <div className="px-3 py-2 max-h-[400px] overflow-y-auto">
            <pre className="text-[0.85em] text-jupiter-muted leading-relaxed whitespace-pre-wrap font-sans">
              {thinking}
            </pre>
          </div>
        </div>
      )}

      {/* Streaming: auto-show last few lines */}
      {streaming && !expanded && (
        <div className="mt-1 rounded-md bg-jupiter-surface/30 border border-jupiter-blue/10 overflow-hidden">
          <div className="px-3 py-1.5">
            <pre className="text-[0.8em] text-jupiter-dim leading-relaxed whitespace-pre-wrap font-sans">
              {lines.slice(-3).join("\n")}
            </pre>
          </div>
        </div>
      )}
    </div>
  );
}
