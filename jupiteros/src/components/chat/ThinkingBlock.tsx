// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

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

  // Minimal: a quiet dim row (no box) + the text as an indented margin note
  // with a thin violet rule — reasoning is context, not content.
  return (
    <div className="mb-2">
      <button
        onClick={() => setExpanded((v) => !v)}
        className="flex items-center gap-1.5 py-1 w-full text-left text-jupiter-dim hover:text-jupiter-muted transition-colors group"
      >
        {/* Icon: animated dots while streaming, neurology glyph when idle */}
        {streaming ? (
          <span className="flex gap-0.5 flex-shrink-0">
            <span className="w-1 h-1 rounded-full bg-jupiter-violet animate-bounce" style={{ animationDelay: "0ms" }} />
            <span className="w-1 h-1 rounded-full bg-jupiter-violet animate-bounce" style={{ animationDelay: "100ms" }} />
            <span className="w-1 h-1 rounded-full bg-jupiter-violet animate-bounce" style={{ animationDelay: "200ms" }} />
          </span>
        ) : (
          <span className="material-symbols-outlined text-[14px] text-jupiter-violet/70 flex-shrink-0">
            neurology
          </span>
        )}

        <span className="text-[0.78em] font-semibold uppercase tracking-wider">
          {streaming ? t("thinking.thinking") : t("thinking.reasoning")}
        </span>

        {!streaming && charCount > 0 && (
          <span className="text-[0.72em] text-jupiter-dim/60">{charLabel}</span>
        )}

        {/* Chevron — hidden while streaming (panel is locked open) */}
        {!streaming && (
          <span className="material-symbols-outlined text-[14px] flex-shrink-0 group-hover:text-jupiter-violet transition-colors">
            {expanded ? "expand_less" : "expand_more"}
          </span>
        )}
      </button>

      {/* Content: indented margin note, scrollable. Auto-scrolls while
          streaming, free-scroll once expanded after completion. */}
      {open && thinking && (
        <div className="mt-1 ml-1.5 border-l-2 border-jupiter-violet/25 pl-3">
          <div ref={scrollRef} className="max-h-[320px] overflow-y-auto custom-scrollbar">
            <pre className="text-[0.82em] text-jupiter-dim leading-relaxed whitespace-pre-wrap font-sans m-0">
              {thinking}
            </pre>
          </div>
        </div>
      )}
    </div>
  );
}
