import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";
import type { ChatMessage as ChatMessageType } from "../../types";
import { ToolTimeline } from "./ToolTimeline";
import { ThinkingBlock } from "./ThinkingBlock";

interface Props {
  message: ChatMessageType;
  onPreviewChart?: (filePath: string) => void;
}

export function ChatMessage({ message, onPreviewChart }: Props) {
  const isUser = message.role === "user";
  const isSystem = message.role === "system";

  if (isSystem) {
    return (
      <div className="flex justify-center">
        <div className="max-w-[90%] rounded-lg px-4 py-2.5 leading-relaxed bg-jupiter-elevated/50 border border-jupiter-dim/20 text-jupiter-muted text-[0.85em]">
          <div className="prose prose-invert max-w-none prose-sm">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{message.content}</ReactMarkdown>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
      <div
        className={`max-w-[85%] rounded-lg px-4 py-2.5 leading-relaxed ${
          isUser
            ? "bg-jupiter-blue/15 text-white border border-jupiter-blue/20"
            : "bg-transparent text-white"
        }`}
      >
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
        {message.content && (
          <div className="prose prose-invert max-w-none">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              components={{
                code({ className, children, ...props }) {
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
                        borderRadius: "0.375rem",
                        fontSize: "0.85em",
                        background: "#0a0a10",
                      }}
                    >
                      {String(children).replace(/\n$/, "")}
                    </SyntaxHighlighter>
                  );
                },
              }}
            >
              {message.content}
            </ReactMarkdown>
          </div>
        )}

        {/* Streaming cursor */}
        {message.streaming && (
          <span className="inline-block w-2 h-4 bg-jupiter-blue animate-pulse ml-0.5" />
        )}
      </div>
    </div>
  );
}
