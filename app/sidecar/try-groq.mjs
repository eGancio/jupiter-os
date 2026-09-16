// Tester CLI per Groq (engine OpenAI-compatibile) coi tool dei Moon.
//   node try-groq.mjs "la tua domanda" [modello]
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";

const here = path.dirname(fileURLToPath(import.meta.url));
const releaseDir = path.resolve(here, "../src-tauri/target/release");

if (!process.env.GROQ_API_KEY) {
  try {
    const line = fs.readFileSync(path.join(releaseDir, "credentials.env"), "utf-8")
      .split(/\r?\n/).find((l) => l.startsWith("GROQ_API_KEY="));
    if (line) process.env.GROQ_API_KEY = line.slice("GROQ_API_KEY=".length).trim();
  } catch { /* ignore */ }
}

const prompt = process.argv[2] || "Quante email ho in totale?";
const model = process.argv[3] || "llama-3.3-70b-versatile";
if (!process.env.GROQ_API_KEY) { console.error("GROQ_API_KEY non trovata (credentials.env o env)."); process.exit(1); }

const { OpenAICompatEngine } = await import("./engines/openai-compat.mjs");
const eng = new OpenAICompatEngine({}, { name: "groq", baseUrl: "https://api.groq.com/openai/v1", keyEnvs: ["GROQ_API_KEY"], maxToolTokens: 3000 });
console.log(`provider: groq | modello: ${model}\n— — —`);
try {
  for await (const ev of eng.run(prompt, { session_id: "cli", model, cwd: releaseDir, mcpConfigPath: path.join(releaseDir, ".mcp.json") })) {
    if (ev.event === "text_delta") process.stdout.write(ev.text);
    else if (ev.event === "tool_start") console.log(`\n[TOOL] ${ev.tool_name}`);
    else if (ev.event === "tool_result") console.log(`[TOOL ←] ${String(ev.result).slice(0, 120).replace(/\s+/g, " ")}…`);
    else if (ev.event === "result") console.log(`\n— — —\n[token in=${ev.usage.input_tokens} out=${ev.usage.output_tokens}]`);
  }
} catch (e) { console.error("\n[errore]", e?.message || e); }
eng.close();
process.exit(0);
