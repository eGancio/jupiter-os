// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import type { ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";
import type { ChatMessage as ChatMessageType, ToolCallInfo } from "../../types";
import { ToolTimeline } from "./ToolTimeline";
import { ThinkingBlock } from "./ThinkingBlock";

interface Props {
  message: ChatMessageType;
  onPreviewChart?: (filePath: string) => void;
}

// Shared markdown renderer config (used by both sequential and legacy paths).
const markdownComponents = {
  code({ className, children, ...props }: any) {
    const match = /language-(\w+)/.exec(className || "");
    const inline = !match;
    return inline ? (
      <code
        className="bg-jupiter-elevated px-1 py-0.5 rounded text-[0.9em]"
        {...props}
      >
        {children}
      </code>
    ) : (
      <SyntaxHighlighter
        style={oneDark}
        language={match[1]}
        PreTag="div"
        customStyle={{
          margin: "0.5rem 0",
          borderRadius: "0.75rem",
          fontSize: "0.85em",
          background: "#0a0a10",
        }}
      >
        {String(children).replace(/\n$/, "")}
      </SyntaxHighlighter>
    );
  },
};

function MarkdownBlock({ text }: { text: string }) {
  return (
    <div className="prose prose-invert max-w-none">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
        {text}
      </ReactMarkdown>
    </div>
  );
}

export function ChatMessage({ message, onPreviewChart }: Props) {
  const isUser = message.role === "user";
  const isSystem = message.role === "system";

  if (isSystem) {
    return (
      <div className="flex justify-center">
        <div className="max-w-[90%] rounded-xl px-4 py-2.5 leading-relaxed bg-jupiter-elevated/50 text-jupiter-muted text-[0.85em] card-shadow">
          <div className="prose prose-invert max-w-none prose-sm">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.content}</ReactMarkdown>
          </div>
        </div>
      </div>
    );
  }

  // Sequential rendering: walk the ordered parts, interleaving thinking / text
  // / tool blocks in arrival order. Consecutive tool parts are coalesced into a
  // single ToolTimeline so the ×N grouping and connecting line are preserved.
  const renderParts = (): ReactNode[] => {
    const parts = message.parts!;
    const out: ReactNode[] = [];
    let toolRun: ToolCallInfo[] = [];

    const flushTools = (key: string) => {
      if (toolRun.length === 0) return;
      const run = toolRun;
      toolRun = [];
      out.push(
        <div key={key} className="mb-2">
          <ToolTimeline toolCalls={run} onPreviewChart={onPreviewChart} />
        </div>
      );
    };

    parts.forEach((part, i) => {
      if (part.kind === "tool") {
        toolRun.push(part.tool);
        return;
      }
      flushTools(`tools-${i}`);
      if (part.kind === "thinking") {
        // Only the trailing thinking block animates while the turn streams.
        const isLast = i === parts.length - 1;
        out.push(
          <ThinkingBlock
            key={`think-${i}`}
            thinking={part.text}
            streaming={message.streaming && isLast}
          />
        );
      } else {
        out.push(<MarkdownBlock key={`text-${i}`} text={part.text} />);
      }
    });
    flushTools("tools-end");
    return out;
  };

  const useSequential = !isUser && !!message.parts && message.parts.length > 0;

  // User → right-aligned bubble with a warm gradient tint and soft shadow.
  // Assistant → gradient orb avatar + soft elevated card, no borders.
  return (
    <div className={`flex gap-3 ${isUser ? "justify-end" : "justify-start"}`}>
      {!isUser && <div className="assistant-orb flex-shrink-0 mt-1" />}
      <div
        className={`max-w-[85%] leading-relaxed ${
          isUser
            ? "rounded-2xl rounded-br-md px-4 py-2.5 text-white bg-gradient-to-br from-jupiter-orange/25 via-jupiter-orange/15 to-jupiter-pink/10 shadow-[0_4px_18px_rgba(0,0,0,0.35)]"
            : "flex-1 min-w-0 rounded-2xl rounded-tl-md px-4 py-3 text-white bg-jupiter-elevated/40 shadow-[0_4px_18px_rgba(0,0,0,0.3)]"
        }`}
      >
        {useSequential ? (
          renderParts()
        ) : (
          <>
            {/* Thinking block (collapsible) */}
            {message.thinking && (
              <ThinkingBlock thinking={message.thinking} streaming={message.streaming} />
            )}

            {/* Tool calls before text — compact timeline */}
            {message.toolCalls.length > 0 && (
              <div className="mb-2">
                <ToolTimeline
                  toolCalls={message.toolCalls}
                  onPreviewChart={onPreviewChart}
                />
              </div>
            )}

            {/* Image previews (user messages) */}
            {message.images && message.images.length > 0 && (
              <div className="flex gap-2 flex-wrap mb-2">
                {message.images.map((img, i) => (
                  <div
                    key={i}
                    className="w-24 h-24 rounded-lg overflow-hidden border border-jupiter-orange/25 bg-jupiter-elevated"
                  >
                    <img
                      src={img.dataUrl}
                      alt={img.name}
                      className="w-full h-full object-cover"
                    />
                  </div>
                ))}
              </div>
            )}

            {/* Message content with markdown */}
            {message.content && <MarkdownBlock text={message.content} />}
          </>
        )}

        {/* Streaming cursor */}
        {message.streaming && (
          <span className="inline-block w-2 h-4 bg-jupiter-orange animate-pulse ml-0.5 rounded-sm" />
        )}
      </div>
    </div>
  );
}
