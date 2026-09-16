// Tester CLI per l'engine Gemini (chat + tool dei Moon).
//   node try-gemini.mjs "la tua domanda" [modello]
// Carica GEMINI_API_KEY da credentials.env e collega i Moon dal .mcp.json se l'app è attiva.
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";

const here = path.dirname(fileURLToPath(import.meta.url));
const releaseDir = path.resolve(here, "../src-tauri/target/release");

if (!process.env.GEMINI_API_KEY && !process.env.GOOGLE_API_KEY) {
  try {
    const line = fs.readFileSync(path.join(releaseDir, "credentials.env"), "utf-8")
      .split(/\r?\n/).find((l) => l.startsWith("GEMINI_API_KEY="));
    if (line) process.env.GEMINI_API_KEY = line.slice("GEMINI_API_KEY=".length).trim();
  } catch { /* ignore */ }
}

const prompt = process.argv[2] || "Ciao! Presentati in una frase.";
const model = process.argv[3] || "gemini-2.5-flash";
if (!process.env.GEMINI_API_KEY) { console.error("GEMINI_API_KEY non trovata."); process.exit(1); }

const mcpConfigPath = path.join(releaseDir, ".mcp.json"); // i Moon, se l'app è avviata
const { GeminiEngine } = await import("./engines/gemini.mjs");
console.log(`modello: ${model}\n— — —`);
const eng = new GeminiEngine();
try {
  for await (const ev of eng.run(prompt, { session_id: "cli", model, cwd: releaseDir, mcpConfigPath })) {
    if (ev.event === "text_delta") process.stdout.write(ev.text);
    else if (ev.event === "tool_start") console.log(`\n[TOOL] ${ev.tool_name}`);
    else if (ev.event === "tool_result") console.log(`[TOOL ←] ${String(ev.result).slice(0, 120).replace(/\s+/g, " ")}…`);
    else if (ev.event === "result") console.log(`\n— — —\n[token in=${ev.usage.input_tokens} out=${ev.usage.output_tokens}]`);
  }
} catch (e) { console.error("\n[errore]", e?.message || e); }
eng.close();
process.exit(0);
