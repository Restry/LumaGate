import { useEffect, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { UpdateView } from "./Updates";

export function useUpdateState(): UpdateView | null {
  const [view, setView] = useState<UpdateView | null>(null);
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let revision = 0;
    void (async () => {
      unlisten = await listen<UpdateView>("manual-update", ({ payload }) => {
        revision += 1;
        if (!disposed) setView(payload);
      });
      if (disposed) {
        unlisten();
        return;
      }
      const started = revision;
      const state = await invoke<UpdateView>("manual_update_state");
      if (!disposed && started === revision) setView(state);
    })().catch(() => {
      /* Silent: update state is optional */
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  return view;
}
