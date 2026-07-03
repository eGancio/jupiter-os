// Dump dei tool MCP reali visti da un engine non-Claude, nello stesso formato
// OpenAI function che usa OpenAICompatEngine. Richiede i Moon in esecuzione.
//
//   node dump-tools.mjs [path/.mcp.json]  →  tools-snapshot.json
//
// Riusa loadMcpServers/relaxSchemaForValidation del sidecar e la denylist
// isUnsafeTool, così lo snapshot coincide con ciò che l'engine esporrebbe.
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import { loadMcpServers, relaxSchemaForValidation } from "../../jupiteros/sidecar/engines/engine.mjs";
import { isUnsafeTool } from "../../jupiteros/sidecar/engines/ollama.mjs";
import { Client } from "../../jupiteros/sidecar/node_modules/@modelcontextprotocol/sdk/dist/esm/client/index.js";
import { SSEClientTransport } from "../../jupiteros/sidecar/node_modules/@modelcontextprotocol/sdk/dist/esm/client/sse.js";
import { StreamableHTTPClientTransport } from "../../jupiteros/sidecar/node_modules/@modelcontextprotocol/sdk/dist/esm/client/streamableHttp.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const mcpConfigPath = process.argv[2]
  || path.join(here, "../../jupiteros/src-tauri/target/release/.mcp.json");

const servers = loadMcpServers(mcpConfigPath);
const out = { mcpConfigPath, dumpedAt: new Date().toISOString(), tools: [], excludedUnsafe: [], skipped: [] };

for (const [serverName, def] of Object.entries(servers)) {
  if (!def.url || (def.type !== "sse" && def.type !== "http")) {
    out.skipped.push({ server: serverName, reason: `transport ${def.type}` });
    continue;
  }
  let client = null;
  try {
    const transport = def.type === "http"
      ? new StreamableHTTPClientTransport(new URL(def.url))
      : new SSEClientTransport(new URL(def.url));
    client = new Client({ name: "localops-eval", version: "0.1.0" }, { capabilities: {} });
    await client.connect(transport);
    const listed = await client.listTools();
    for (const t of listed?.tools || []) {
      if (isUnsafeTool(t.name)) { out.excludedUnsafe.push(`${serverName}:${t.name}`); continue; }
      out.tools.push({
        server: serverName,
        type: "function",
        function: {
          name: t.name,
          description: t.description || "",
          parameters: relaxSchemaForValidation(t.inputSchema || { type: "object", properties: {} }),
        },
      });
    }
    console.error(`${serverName}: ${listed?.tools?.length ?? 0} tool (${out.tools.filter(x => x.server === serverName).length} inclusi)`);
  } catch (e) {
    out.skipped.push({ server: serverName, reason: String(e?.message || e) });
    console.error(`${serverName}: NON raggiungibile — ${e?.message || e}`);
  } finally {
    try { await client?.close?.(); } catch { /* ignore */ }
  }
}

const dest = path.join(here, "tools-snapshot.json");
fs.writeFileSync(dest, JSON.stringify(out, null, 2));
console.error(`\n${out.tools.length} tool inclusi, ${out.excludedUnsafe.length} esclusi (denylist), ${out.skipped.length} server saltati → ${dest}`);
