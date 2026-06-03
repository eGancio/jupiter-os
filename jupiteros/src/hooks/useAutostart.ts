// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useState, useEffect } from "react";
import {
  isAutostartEnabled,
  setAutostart as setAutostartCmd,
} from "../lib/tauri";

export function useAutostart() {
  const [enabled, setEnabled] = useState(false);

  useEffect(() => {
    isAutostartEnabled().then(setEnabled);
  }, []);

  const toggle = async () => {
    const newValue = !enabled;
    await setAutostartCmd(newValue);
    setEnabled(newValue);
  };

  return { enabled, toggle };
}
