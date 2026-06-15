// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import type { ChatToast } from "../../types";
import { useT } from "../../i18n";

interface Props {
  toasts: ChatToast[];
  onDismiss: (id: number) => void;
  /** Activate (and reopen if needed) the chat the toast refers to. */
  onActivate: (sessionId: string) => void;
}

function toastText(t: (k: string, p?: Record<string, string>) => string, toast: ChatToast): string {
  const title = toast.title || t("chatlist.untitled");
  switch (toast.kind) {
    case "done":
      return t("toast.done", { title });
    case "error":
      return t("toast.error", { title });
    case "info":
      return t("tabs.limit", { max: "4" });
  }
}

/** Clickable notification stack for background-chat events (bottom-right). */
export function Toasts({ toasts, onDismiss, onActivate }: Props) {
  const { t } = useT();
  if (toasts.length === 0) return null;

  return (
    <div className="fixed bottom-4 right-4 z-50 flex flex-col gap-2 items-end">
      {toasts.map((toast) => {
        const clickable = toast.kind !== "info";
        const accent =
          toast.kind === "error"
            ? "border-red-500/60"
            : toast.kind === "done"
              ? "border-jupiter-green/60"
              : "border-jupiter-orange/40";
        return (
          <div
            key={toast.id}
            role={clickable ? "button" : "status"}
            onClick={() => {
              if (clickable) {
                onActivate(toast.sessionId);
                onDismiss(toast.id);
              }
            }}
            className={`flex items-center gap-3 pl-3 pr-2 py-2 rounded-lg border ${accent} bg-jupiter-elevated/95 backdrop-blur text-[12px] text-white shadow-lg max-w-[320px] ${
              clickable ? "cursor-pointer hover:bg-jupiter-elevated" : ""
            }`}
          >
            <span
              className={`w-2 h-2 rounded-full flex-shrink-0 ${
                toast.kind === "error"
                  ? "bg-red-500"
                  : toast.kind === "done"
                    ? "bg-jupiter-green"
                    : "bg-jupiter-orange"
              }`}
            />
            <span className="truncate">{toastText(t, toast)}</span>
            <button
              onClick={(e) => {
                e.stopPropagation();
                onDismiss(toast.id);
              }}
              title={t("toast.dismiss")}
              className="flex-shrink-0 rounded px-1 text-jupiter-dim hover:text-white"
            >
              ×
            </button>
          </div>
        );
      })}
    </div>
  );
}
