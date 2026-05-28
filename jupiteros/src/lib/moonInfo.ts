export interface MoonTool {
  name: string;
  description: string;
}

export interface MoonInfo {
  displayName: string;
  category: string;
  description: string;
  port: number;
  location: string;
  tools: MoonTool[];
}

const MOON_INFO: Record<string, MoonInfo> = {
  io: {
    displayName: "Moon Io",
    category: "Email & Calendar",
    description:
      "Handles all email operations via IMAP/SMTP/CalDAV, including multi-account support (Aruba, Gmail OAuth), semantic search with Qdrant + ONNX embeddings, calendar events, and file attachment extraction.",
    port: 8100,
    location: "Local · Rust",
    tools: [
      { name: "list_emails", description: "List email headers (fast, default)" },
      { name: "read_recent_emails", description: "Read email bodies (max 5)" },
      { name: "search_emails", description: "Semantic search across emails" },
      { name: "search_by_contact", description: "All emails with a contact" },
      { name: "send_email", description: "Send a new email (requires draft approval)" },
      { name: "reply_email", description: "Reply in thread (requires draft approval)" },
      { name: "save_attachment", description: "Download an email attachment" },
      { name: "get_email_attachments", description: "List attachments of an email" },
      { name: "extract_file_text", description: "Extract text from PDF/DOCX/XLSX" },
      { name: "list_calendar_events", description: "List upcoming calendar events" },
      { name: "create_calendar_event", description: "Create a calendar event" },
      { name: "get_email_signature", description: "Show current email signature" },
      { name: "set_email_signature", description: "Set email signature" },
      { name: "get_stats", description: "Mailbox statistics" },
    ],
  },
  europa: {
    displayName: "Moon Europa",
    category: "Messaging",
    description:
      "Handles Telegram messaging via native MTProto (grammers), with semantic search powered by Qdrant + ONNX embeddings. Future support for WhatsApp and Discord planned.",
    port: 8200,
    location: "Local · Rust",
    tools: [
      { name: "list_messages", description: "List message headers (fast, default)" },
      { name: "read_messages", description: "Read message bodies (max 5)" },
      { name: "search_messages", description: "Semantic search across messages" },
      { name: "search_by_contact", description: "All messages with a contact" },
      { name: "send_message", description: "Send a message (requires approval)" },
      { name: "list_contacts", description: "List available Telegram contacts" },
      { name: "save_media", description: "Download media from a chat" },
      { name: "extract_file_text", description: "Extract text from PDF/DOCX/XLSX" },
      { name: "get_stats", description: "Messaging statistics" },
    ],
  },
  amalthea: {
    displayName: "Moon Amalthea",
    category: "Charts & Diagrams",
    description:
      "Generates 16 types of deterministic charts and diagrams using ECharts and Mermaid. Returns self-contained HTML files that render in the JupiterOS chart panel.",
    port: 8300,
    location: "Local · Python",
    tools: [
      { name: "bar_chart", description: "Vertical or horizontal bar chart" },
      { name: "line_chart", description: "Line chart with optional multi-series" },
      { name: "pie_chart", description: "Pie or donut chart" },
      { name: "scatter_chart", description: "Scatter / bubble chart" },
      { name: "heatmap", description: "Calendar or matrix heatmap" },
      { name: "treemap", description: "Hierarchical treemap" },
      { name: "funnel_chart", description: "Funnel / conversion chart" },
      { name: "gauge_chart", description: "Gauge / speedometer" },
      { name: "radar_chart", description: "Radar / spider chart" },
      { name: "candlestick_chart", description: "OHLC candlestick chart" },
      { name: "sankey_diagram", description: "Sankey flow diagram" },
      { name: "graph_network", description: "Force-directed network graph" },
      { name: "mermaid_flowchart", description: "Mermaid flowchart" },
      { name: "mermaid_sequence", description: "Mermaid sequence diagram" },
      { name: "mermaid_gantt", description: "Mermaid Gantt chart" },
      { name: "mermaid_er", description: "Mermaid entity-relationship diagram" },
    ],
  },
};

const MOON_COLORS: Record<string, string> = {
  io: "#f59e0b",
  europa: "#3b82f6",
  amalthea: "#8b5cf6",
};

export function getMoonInfo(moonName: string): MoonInfo | null {
  return MOON_INFO[moonName.toLowerCase()] ?? null;
}

export function getMoonColor(moonName: string): string {
  return MOON_COLORS[moonName.toLowerCase()] ?? "#f97316";
}
