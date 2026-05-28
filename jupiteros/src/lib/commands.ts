export interface SlashCommand {
  name: string;
  description: string;
  usage?: string;
}

export const COMMANDS: SlashCommand[] = [
  {
    name: "/clear",
    description: "Clear current chat history",
  },
  {
    name: "/compact",
    description: "Summarise conversation and start a fresh context window",
  },
  {
    name: "/model",
    description: "Switch Claude model",
    usage: "/model claude-opus-4-5",
  },
  {
    name: "/cost",
    description: "Show token usage and cost for this session",
  },
  {
    name: "/mcp",
    description: "List connected MCP servers and their tools",
  },
  {
    name: "/plan",
    description: "Toggle plan mode (Claude proposes before acting)",
  },
];

export function getCompletions(text: string): SlashCommand[] {
  const lower = text.toLowerCase();
  return COMMANDS.filter((cmd) => cmd.name.startsWith(lower));
}
