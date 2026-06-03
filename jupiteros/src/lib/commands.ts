// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

export interface SlashCommand {
  name: string;
  /** i18n key for the command description (resolve with the t() helper). */
  descKey: string;
  usage?: string;
}

export const COMMANDS: SlashCommand[] = [
  {
    name: "/clear",
    descKey: "cmd.clear.desc",
  },
  {
    name: "/compact",
    descKey: "cmd.compact.desc",
  },
  {
    name: "/model",
    descKey: "cmd.model.desc",
    usage: "/model claude-opus-4-5",
  },
  {
    name: "/cost",
    descKey: "cmd.cost.desc",
  },
  {
    name: "/mcp",
    descKey: "cmd.mcp.desc",
  },
  {
    name: "/plan",
    descKey: "cmd.plan.desc",
  },
];

export function getCompletions(text: string): SlashCommand[] {
  const lower = text.toLowerCase();
  return COMMANDS.filter((cmd) => cmd.name.startsWith(lower));
}
