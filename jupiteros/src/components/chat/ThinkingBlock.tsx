import { useEffect, useRef, useState } from "react";
import { useT } from "../../i18n";

interface Props {
  thinking: string;
  streaming?: boolean;
}

export function ThinkingBlock({ thinking, streaming }: Props) {
  const { t } = useT();
  const [expanded, setExpanded] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  // While streaming the panel is always open and follows the latest text.
  // Once the turn ends it collapses to a tidy header; the user can re-open it.
  const open = streaming || expanded;

  const charCount = thinking.length;
  const charLabel =
    charCount > 1000
      ? t("thinking.kchars", { count: Math.round(charCount / 1000) })
      : t("thinking.chars", { count: charCount });

  // Auto-scroll to the bottom as new thinking text arrives during streaming.
  useEffect(() => {
    if (streaming && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [thinking, streaming]);

  return (
    <div className="mb-2">
      <button
        onClick={() => setExpanded((v) => !v)}
        className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-jupiter-surface/60 border border-jupiter-blue/20 hover:border-jupiter-blue/40 transition-colors w-full text-left group"
      >
        {/* Icon: animated dots while streaming, brain when idle */}
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
          {streaming ? t("thinking.thinking") : t("thinking.reasoning")}
        </span>

        {!streaming && charCount > 0 && (
          <span className="text-[0.8em] text-jupiter-dim ml-1">{charLabel}</span>
        )}

        {/* Chevron — hidden while streaming (panel is locked open) */}
        {!streaming && (
          <span className="ml-auto text-jupiter-dim text-[0.8em] flex-shrink-0 group-hover:text-jupiter-blue transition-colors">
            {expanded ? "▾" : "▸"}
          </span>
        )}
      </button>

      {/* Content panel: full thinking text, scrollable. Auto-scrolls while
          streaming, free-scroll once expanded after completion. */}
      {open && thinking && (
        <div className="mt-1 rounded-md bg-jupiter-surface/40 border border-jupiter-blue/15 overflow-hidden">
          <div ref={scrollRef} className="px-3 py-2 max-h-[320px] overflow-y-auto">
            <pre className="text-[0.85em] text-jupiter-muted leading-relaxed whitespace-pre-wrap font-sans m-0">
              {thinking}
            </pre>
          </div>
        </div>
      )}
    </div>
  );
}
