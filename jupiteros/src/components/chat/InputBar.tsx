// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useState, useRef, useCallback, useMemo, type ReactNode } from "react";
import { readImage } from "@tauri-apps/plugin-clipboard-manager";
import type { PastedImage } from "../../types";
import { getCompletions, type SlashCommand } from "../../lib/commands";
import { useT } from "../../i18n";

type PermissionMode = "auto" | "plan" | "bypass";

interface Props {
  onSend: (text: string, images?: PastedImage[]) => void;
  onStop: () => void;
  onCommand?: (name: string, args: string) => void;
  disabled: boolean;
  streaming: boolean;
  zoom?: number;
  permissionMode?: PermissionMode;
  onPermissionModeChange?: () => void;
  showPermissionToggle?: boolean;
  activeTool?: string | null;
  contextPercent?: number;
  activeFile?: string | null;
  /** Model picker rendered in the bottom-left bar (next to "Auto mode"). */
  modelSlot?: ReactNode;
}

const PERMISSION_LABEL_KEYS: Record<PermissionMode, string> = {
  auto: "input.permission.auto",
  plan: "input.permission.plan",
  bypass: "input.permission.bypass",
};

export function InputBar({ onSend, onStop, onCommand, disabled: _disabled, streaming, zoom = 1, permissionMode = "auto", showPermissionToggle = true, activeTool, activeFile, modelSlot }: Props) {
  const { t } = useT();
  const [text, setText] = useState("");
  const [images, setImages] = useState<PastedImage[]>([]);
  const [selectedCompletion, setSelectedCompletion] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Slash command autocomplete (hide when text is an exact match — let Enter execute it)
  const completions = useMemo(() => {
    const trimmed = text.trim();
    if (!trimmed.startsWith("/") || trimmed.includes(" ")) return [];
    const matches = getCompletions(trimmed);
    if (matches.length === 1 && matches[0].name === trimmed.toLowerCase()) return [];
    return matches;
  }, [text]);

  const handleSend = useCallback(() => {
    const trimmed = text.trim();
    if (!trimmed && images.length === 0) return;

    // Check for slash command
    if (trimmed.startsWith("/") && onCommand) {
      const spaceIdx = trimmed.indexOf(" ");
      const name = spaceIdx >= 0 ? trimmed.slice(0, spaceIdx).toLowerCase() : trimmed.toLowerCase();
      const args = spaceIdx >= 0 ? trimmed.slice(spaceIdx + 1).trim() : "";
      onCommand(name, args);
      setText("");
      if (textareaRef.current) textareaRef.current.style.height = "auto";
      return;
    }

    onSend(trimmed, images.length > 0 ? images : undefined);
    setText("");
    setImages([]);
    if (textareaRef.current) {
      textareaRef.current.style.height = "auto";
    }
  }, [text, images, onSend, onCommand]);

  const acceptCompletion = useCallback((cmd: SlashCommand) => {
    setText(cmd.name + " ");
    setSelectedCompletion(0);
    textareaRef.current?.focus();
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      // Navigate autocomplete
      if (completions.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setSelectedCompletion((prev) => Math.min(prev + 1, completions.length - 1));
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setSelectedCompletion((prev) => Math.max(prev - 1, 0));
          return;
        }
        if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
          e.preventDefault();
          acceptCompletion(completions[selectedCompletion]);
          return;
        }
        if (e.key === "Escape") {
          setText("");
          return;
        }
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend, completions, selectedCompletion, acceptCompletion]
  );

  const handleInput = useCallback(() => {
    const el = textareaRef.current;
    if (el) {
      el.style.height = "auto";
      el.style.height = Math.min(el.scrollHeight, 150) + "px";
    }
  }, []);

  const addImageFromFile = useCallback((file: File) => {
    if (!file.type.startsWith("image/")) return;
    const reader = new FileReader();
    reader.onload = (e) => {
      const dataUrl = e.target?.result as string;
      setImages((prev) => [
        ...prev,
        { id: `img-${Date.now()}-${Math.random()}`, dataUrl, name: file.name },
      ]);
    };
    reader.readAsDataURL(file);
  }, []);

  const addImageFromDataUrl = useCallback((dataUrl: string, name: string) => {
    setImages((prev) => [
      ...prev,
      { id: `img-${Date.now()}-${Math.random()}`, dataUrl, name },
    ]);
  }, []);

  // Read an image off the system clipboard via Tauri and convert the raw RGBA
  // to a PNG data URL. WebKitGTK (Tauri/Linux) does NOT expose pasted images in
  // the web `clipboardData`, so the standard paste path silently finds nothing —
  // this is the only reliable way to support Ctrl+V of an image on Linux.
  const tryPasteFromTauri = useCallback(async () => {
    try {
      const image = await readImage();
      const rgba = await image.rgba();
      const { width, height } = await image.size();
      if (!width || !height) return false;
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const ctx = canvas.getContext("2d");
      if (!ctx) return false;
      ctx.putImageData(
        new ImageData(new Uint8ClampedArray(rgba), width, height),
        0,
        0,
      );
      addImageFromDataUrl(canvas.toDataURL("image/png"), `pasted-${Date.now()}.png`);
      return true;
    } catch {
      // Clipboard holds no image (e.g. a text paste) — nothing to do.
      return false;
    }
  }, [addImageFromDataUrl]);

  const handlePaste = useCallback(
    (e: React.ClipboardEvent) => {
      const items = e.clipboardData?.items;
      // Index-based: on WebKitGTK (Tauri/Linux) DataTransferItemList is not
      // iterable with for...of, so a for-of loop throws and the paste is lost.
      if (items) {
        for (let i = 0; i < items.length; i++) {
          const item = items[i];
          if (item.type.startsWith("image/")) {
            e.preventDefault();
            const file = item.getAsFile();
            if (file) addImageFromFile(file);
            return;
          }
        }
      }
      // No image in the web clipboard (the WebKitGTK case): fall back to reading
      // the system clipboard through Tauri. Fire-and-forget — if it finds only
      // text, the default text paste already happened, which is what we want.
      void tryPasteFromTauri();
    },
    [addImageFromFile, tryPasteFromTauri]
  );

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      const files = e.dataTransfer?.files;
      if (!files) return;
      // Index-based: FileList isn't reliably iterable on WebKitGTK either.
      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        if (file.type.startsWith("image/")) {
          addImageFromFile(file);
        }
      }
    },
    [addImageFromFile]
  );

  const removeImage = useCallback((id: string) => {
    setImages((prev) => prev.filter((img) => img.id !== id));
  }, []);

  const handleAttachClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = e.target.files;
      if (!files) return;
      for (let i = 0; i < files.length; i++) {
        addImageFromFile(files[i]);
      }
      e.target.value = "";
    },
    [addImageFromFile]
  );

  /** Friendly tool display name */
  const toolDisplayName = activeTool
    ? activeTool.split("__").pop() || activeTool
    : null;

  const hasContent = text.trim().length > 0 || images.length > 0;
  const planMode = permissionMode === "plan";

  return (
    <div className="px-3 pb-3 pt-2 flex-shrink-0">
      {/* Slash command autocomplete dropdown */}
      {completions.length > 0 && (
        <div className="mx-3 mb-1 rounded-lg border border-jupiter-orange/30 bg-jupiter-surface overflow-hidden shadow-lg">
          {completions.map((cmd, i) => (
            <button
              key={cmd.name}
              className={`w-full text-left px-3 py-1.5 text-[12px] flex items-center gap-2 transition-colors ${
                i === selectedCompletion
                  ? "bg-jupiter-orange/15 text-jupiter-orange"
                  : "text-jupiter-muted hover:bg-jupiter-elevated"
              }`}
              onMouseDown={(e) => {
                e.preventDefault();
                acceptCompletion(cmd);
              }}
              onMouseEnter={() => setSelectedCompletion(i)}
            >
              <span className="font-mono text-jupiter-amber">{cmd.name}</span>
              <span className="text-jupiter-dim">{t(cmd.descKey)}</span>
            </button>
          ))}
        </div>
      )}

      <div
        className={`rounded-xl border-2 bg-jupiter-surface overflow-hidden transition-colors ${
          planMode
            ? "border-jupiter-blue/60 focus-within:border-jupiter-blue"
            : "border-jupiter-orange/60 focus-within:border-jupiter-orange"
        }`}
        onDrop={handleDrop}
        onDragOver={(e) => e.preventDefault()}
      >
        {/* Textarea */}
        <textarea
          ref={textareaRef}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={handleKeyDown}
          onInput={handleInput}
          onPaste={handlePaste}
          placeholder={planMode ? t("input.placeholderPlan") : t("input.placeholder")}
          rows={1}
          style={{ fontSize: `${14 * zoom}px` }}
          className="w-full bg-transparent text-white outline-none placeholder:text-jupiter-dim resize-none overflow-hidden px-4 pt-3 pb-1"
        />

        {/* Image previews */}
        {images.length > 0 && (
          <div className="flex gap-2 px-4 pb-2 flex-wrap">
            {images.map((img) => (
              <div
                key={img.id}
                className="relative group w-16 h-16 rounded-lg overflow-hidden border border-jupiter-orange/25 bg-jupiter-elevated"
              >
                <img
                  src={img.dataUrl}
                  alt={img.name}
                  className="w-full h-full object-cover"
                />
                <button
                  onClick={() => removeImage(img.id)}
                  className="absolute top-0.5 right-0.5 w-4 h-4 rounded-full bg-black/70 text-white text-[10px] flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity"
                >
                  x
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Bottom bar — status left, actions right (VS Code Claude Code style) */}
        <div className="flex items-center justify-between px-3 py-1.5 border-t border-white/5">
          {/* Left — status indicators */}
          <div className="flex items-center gap-3 text-[12px] min-w-0 overflow-hidden">
            {/* Model / engine picker */}
            {modelSlot}

            {/* Permission mode — single real "Auto mode", Claude only */}
            {showPermissionToggle && (
              <span className="flex items-center gap-1.5 whitespace-nowrap text-white/60">
                <span className="text-[10px]">&#9654;&#9654;</span>
                <span>{t(PERMISSION_LABEL_KEYS[permissionMode])}</span>
              </span>
            )}

            {/* Active tool + file context */}
            {(toolDisplayName || activeFile) && (
              <span className="flex items-center gap-1.5 text-white/60 whitespace-nowrap min-w-0">
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="flex-shrink-0">
                  <polyline points="16 18 22 12 16 6" />
                  <polyline points="8 6 2 12 8 18" />
                </svg>
                <span className="truncate">
                  {toolDisplayName || ""}{activeFile ? ` ${activeFile}` : ""}
                </span>
              </span>
            )}

          </div>

          {/* Right — action buttons */}
          <div className="flex items-center gap-1 flex-shrink-0">
            <input
              ref={fileInputRef}
              type="file"
              accept="image/*"
              multiple
              className="hidden"
              onChange={handleFileChange}
            />
            <button
              onClick={handleAttachClick}
              className="w-7 h-7 flex items-center justify-center text-white/30 hover:text-white rounded-md hover:bg-white/10 transition-colors"
              title={t("input.attach")}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M21.44 11.05l-9.19 9.19a6 6 0 01-8.49-8.49l9.19-9.19a4 4 0 015.66 5.66l-9.2 9.19a2 2 0 01-2.83-2.83l8.49-8.48" />
              </svg>
            </button>

            {/* Slash commands shortcut */}
            <button
              onClick={() => { setText("/"); textareaRef.current?.focus(); }}
              className="w-7 h-7 flex items-center justify-center text-white/30 hover:text-white rounded-md hover:bg-white/10 transition-colors text-[14px] font-bold"
              title={t("input.slash")}
            >
              /
            </button>

            {/* Stop button (only while streaming) */}
            {streaming && (
              <button
                onClick={onStop}
                className="w-7 h-7 flex items-center justify-center rounded-lg bg-jupiter-red/80 text-white hover:bg-jupiter-red transition-colors"
                title={t("input.stop")}
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                  <rect x="4" y="4" width="16" height="16" rx="2" />
                </svg>
              </button>
            )}

            {/* Send button */}
            <button
              onClick={handleSend}
              disabled={!hasContent}
              className={`w-7 h-7 flex items-center justify-center rounded-lg text-white disabled:opacity-30 disabled:cursor-not-allowed transition-colors ${
                planMode
                  ? "bg-jupiter-blue hover:bg-jupiter-blue/80"
                  : "bg-jupiter-orange hover:bg-jupiter-orange-light glow-orange"
              }`}
              title={t("input.send")}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <line x1="12" y1="19" x2="12" y2="5" />
                <polyline points="5 12 12 5 19 12" />
              </svg>
            </button>
          </div>
        </div>
      </div>

    </div>
  );
}
