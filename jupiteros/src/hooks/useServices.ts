import { useState, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { getServices, getVirtualMoons } from "../lib/tauri";
import type { ServiceInfo, StatusChangedEvent } from "../types";

export function useServices() {
  const [services, setServices] = useState<ServiceInfo[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    const [local, virtual] = await Promise.all([getServices(), getVirtualMoons()]);
    const virtualTagged = virtual.map((s) => ({ ...s, virtual: true }));
    setServices([...local, ...virtualTagged]);
    setLoading(false);
  }, []);

  useEffect(() => {
    refresh();

    const unlistenPromise = listen<StatusChangedEvent[]>(
      "service-status-changed",
      (event) => {
        setServices((prev) =>
          prev.map((s) => {
            const changed = event.payload.find((c) => c.name === s.name);
            return changed ? { ...s, running: changed.running } : s;
          })
        );
      }
    );

    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, [refresh]);

  return { services, loading, refresh };
}
