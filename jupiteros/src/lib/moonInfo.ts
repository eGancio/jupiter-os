// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

export interface MoonTool {
  name: string;
  /** i18n key for the tool description (resolve with the t() helper). */
  descKey: string;
}

export interface MoonInfo {
  displayName: string;
  /** i18n key for the category label. */
  categoryKey: string;
  /** i18n key for the description paragraph. */
  descKey: string;
  port: number;
  location: string;
  tools: MoonTool[];
}

const MOON_INFO: Record<string, MoonInfo> = {
  io: {
    displayName: "Moon Io",
    categoryKey: "moon.io.category",
    descKey: "moon.io.desc",
    port: 8100,
    location: "Local · Rust",
    tools: [
      { name: "list_emails", descKey: "moontool.io.list_emails" },
      { name: "read_recent_emails", descKey: "moontool.io.read_recent_emails" },
      { name: "search_emails", descKey: "moontool.io.search_emails" },
      { name: "search_by_contact", descKey: "moontool.io.search_by_contact" },
      { name: "send_email", descKey: "moontool.io.send_email" },
      { name: "reply_email", descKey: "moontool.io.reply_email" },
      { name: "save_attachment", descKey: "moontool.io.save_attachment" },
      { name: "get_email_attachments", descKey: "moontool.io.get_email_attachments" },
      { name: "extract_file_text", descKey: "moontool.io.extract_file_text" },
      { name: "list_calendar_events", descKey: "moontool.io.list_calendar_events" },
      { name: "create_calendar_event", descKey: "moontool.io.create_calendar_event" },
      { name: "get_email_signature", descKey: "moontool.io.get_email_signature" },
      { name: "set_email_signature", descKey: "moontool.io.set_email_signature" },
      { name: "get_stats", descKey: "moontool.io.get_stats" },
    ],
  },
  europa: {
    displayName: "Moon Europa",
    categoryKey: "moon.europa.category",
    descKey: "moon.europa.desc",
    port: 8200,
    location: "Local · Rust",
    tools: [
      { name: "list_messages", descKey: "moontool.europa.list_messages" },
      { name: "read_messages", descKey: "moontool.europa.read_messages" },
      { name: "search_messages", descKey: "moontool.europa.search_messages" },
      { name: "search_by_contact", descKey: "moontool.europa.search_by_contact" },
      { name: "send_message", descKey: "moontool.europa.send_message" },
      { name: "list_contacts", descKey: "moontool.europa.list_contacts" },
      { name: "save_media", descKey: "moontool.europa.save_media" },
      { name: "extract_file_text", descKey: "moontool.europa.extract_file_text" },
      { name: "get_stats", descKey: "moontool.europa.get_stats" },
    ],
  },
  amalthea: {
    displayName: "Moon Amalthea",
    categoryKey: "moon.amalthea.category",
    descKey: "moon.amalthea.desc",
    port: 8300,
    location: "Local · Python",
    tools: [
      { name: "bar_chart", descKey: "moontool.amalthea.bar_chart" },
      { name: "line_chart", descKey: "moontool.amalthea.line_chart" },
      { name: "pie_chart", descKey: "moontool.amalthea.pie_chart" },
      { name: "scatter_chart", descKey: "moontool.amalthea.scatter_chart" },
      { name: "heatmap", descKey: "moontool.amalthea.heatmap" },
      { name: "treemap", descKey: "moontool.amalthea.treemap" },
      { name: "funnel_chart", descKey: "moontool.amalthea.funnel_chart" },
      { name: "gauge_chart", descKey: "moontool.amalthea.gauge_chart" },
      { name: "radar_chart", descKey: "moontool.amalthea.radar_chart" },
      { name: "candlestick_chart", descKey: "moontool.amalthea.candlestick_chart" },
      { name: "sankey_diagram", descKey: "moontool.amalthea.sankey_diagram" },
      { name: "graph_network", descKey: "moontool.amalthea.graph_network" },
      { name: "mermaid_flowchart", descKey: "moontool.amalthea.mermaid_flowchart" },
      { name: "mermaid_sequence", descKey: "moontool.amalthea.mermaid_sequence" },
      { name: "mermaid_gantt", descKey: "moontool.amalthea.mermaid_gantt" },
      { name: "mermaid_er", descKey: "moontool.amalthea.mermaid_er" },
    ],
  },
  metis: {
    displayName: "Moon Metis",
    categoryKey: "moon.metis.category",
    descKey: "moon.metis.desc",
    port: 8600,
    location: "Local · Rust",
    tools: [
      { name: "metis_preview", descKey: "moontool.metis.metis_preview" },
      { name: "metis_ingest", descKey: "moontool.metis.metis_ingest" },
      { name: "metis_ingest_folder", descKey: "moontool.metis.metis_ingest_folder" },
      { name: "metis_search", descKey: "moontool.metis.metis_search" },
      { name: "metis_list_documents", descKey: "moontool.metis.metis_list_documents" },
      { name: "metis_doc_info", descKey: "moontool.metis.metis_doc_info" },
      { name: "metis_get_chunk", descKey: "moontool.metis.metis_get_chunk" },
      { name: "metis_classify_nature", descKey: "moontool.metis.metis_classify_nature" },
    ],
  },
  himalia: {
    displayName: "Moon Himalia",
    categoryKey: "moon.himalia.category",
    descKey: "moon.himalia.desc",
    port: 8700,
    location: "Local · Rust",
    tools: [
      { name: "web_search", descKey: "moontool.himalia.web_search" },
      { name: "web_extract", descKey: "moontool.himalia.web_extract" },
    ],
  },
  elara: {
    displayName: "Moon Elara",
    categoryKey: "moon.elara.category",
    descKey: "moon.elara.desc",
    port: 8800,
    location: "Local · Rust",
    tools: [
      { name: "news_search", descKey: "moontool.elara.news_search" },
      { name: "reddit_search", descKey: "moontool.elara.reddit_search" },
    ],
  },
};

const MOON_COLORS: Record<string, string> = {
  io: "#f59e0b",
  europa: "#3b82f6",
  amalthea: "#8b5cf6",
  metis: "#06b6d4",
  himalia: "#10b981",
  elara: "#ec4899",
};

export function getMoonInfo(moonName: string): MoonInfo | null {
  return MOON_INFO[moonName.toLowerCase()] ?? null;
}

export function getMoonColor(moonName: string): string {
  return MOON_COLORS[moonName.toLowerCase()] ?? "#f97316";
}
