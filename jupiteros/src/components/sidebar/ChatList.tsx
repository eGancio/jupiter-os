// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useState, useRef, useEffect } from "react";
import { useT } from "../../i18n";
import type { ChatSessionInfo } from "../../types";

interface Props {
  sessions: ChatSessionInfo[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onDelete: (id: string) => void;
  onRename: (id: string, title: string) => void;
}

export function ChatList({ sessions, activeId, onSelect, onNew, onDelete, onRename }: Props) {
  const { t } = useT();
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editingId && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editingId]);

  const startRename = (id: string, currentTitle: string) => {
    setEditingId(id);
    setEditValue(currentTitle);
  };

  const commitRename = () => {
    if (editingId && editValue.trim()) {
      onRename(editingId, editValue.trim());
    }
    setEditingId(null);
  };

  return (
    <div className="mb-1">
      <div className="flex items-center justify-between px-3 mb-1">
        <h3 className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim">
          {t("chatlist.title")}
        </h3>
        <button
          onClick={onNew}
          className="w-5 h-5 flex items-center justify-center text-[13px] text-jupiter-dim hover:text-jupiter-orange rounded transition-colors"
          title={t("chatlist.new")}
        >
          +
        </button>
      </div>
      <div className="px-2 max-h-[200px] overflow-y-auto space-y-0.5">
        {sessions.length === 0 ? (
          <div className="px-3 py-1.5 text-[11px] text-jupiter-dim italic">
            {t("chatlist.empty")}
          </div>
        ) : (
          sessions.map((s) => {
            const isActive = s.id === activeId;
            const isEditing = s.id === editingId;

            return (
              <div
                key={s.id}
                className={`group flex items-center gap-1.5 px-2 py-1.5 rounded text-[12px] cursor-pointer transition-colors ${
                  isActive
                    ? "bg-jupiter-orange/15 text-jupiter-orange border border-jupiter-orange/30"
                    : "text-jupiter-muted hover:bg-jupiter-elevated hover:text-white border border-transparent"
                }`}
                onClick={() => !isEditing && onSelect(s.id)}
              >
                <span
                  className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${
                    isActive ? "bg-jupiter-orange" : "bg-jupiter-dim"
                  }`}
                />

                {isEditing ? (
                  <input
                    ref={inputRef}
                    value={editValue}
                    onChange={(e) => setEditValue(e.target.value)}
                    onBlur={commitRename}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename();
                      if (e.key === "Escape") setEditingId(null);
                    }}
                    className="flex-1 min-w-0 bg-jupiter-bg border border-jupiter-orange/30 rounded px-1 py-0 text-[12px] text-white outline-none"
                    onClick={(e) => e.stopPropagation()}
                  />
                ) : (
                  <span
                    className="flex-1 min-w-0 truncate"
                    onDoubleClick={(e) => {
                      e.stopPropagation();
                      startRename(s.id, s.title);
                    }}
                  >
                    {s.title || t("chatlist.untitled")}
                  </span>
                )}

                {/* Actions (visible on hover) */}
                {!isEditing && (
                  <div className="flex-shrink-0 flex gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        startRename(s.id, s.title);
                      }}
                      className="w-4 h-4 flex items-center justify-center text-[10px] text-jupiter-dim hover:text-jupiter-blue rounded"
                      title={t("chatlist.rename")}
                    >
                      &#9998;
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        onDelete(s.id);
                      }}
                      className="w-4 h-4 flex items-center justify-center text-[10px] text-jupiter-dim hover:text-jupiter-red rounded"
                      title={t("chatlist.delete")}
                    >
                      &#10005;
                    </button>
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
