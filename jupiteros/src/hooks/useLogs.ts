// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { getLogs, clearLogs as clearLogsCmd } from "../lib/tauri";
import type { LogLineEvent } from "../types";

export function useLogs(serviceName: string | null) {
  const [lines, setLines] = useState<string[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Load existing logs when service is selected
  useEffect(() => {
    if (!serviceName) {
      setLines([]);
      return;
    }
    getLogs(serviceName).then((data) => setLines(data.lines));
  }, [serviceName]);

  // Stream new log lines via events
  useEffect(() => {
    if (!serviceName) return;

    const unlistenPromise = listen<LogLineEvent>("log-line", (event) => {
      if (event.payload.service === serviceName) {
        setLines((prev) => {
          const next = [...prev, event.payload.line];
          return next.length > 500 ? next.slice(next.length - 500) : next;
        });
      }
    });

    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, [serviceName]);

  // Auto-scroll to bottom
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [lines]);

  const clearLogs = async () => {
    if (serviceName) {
      await clearLogsCmd(serviceName);
      setLines([]);
    }
  };

  return { lines, scrollRef, clearLogs };
}
