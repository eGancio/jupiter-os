# JupiterOS — Landing Page Brief

> **Per chi sviluppa la landing**: questo documento è autosufficiente. Tutto il contenuto, il design e le specifiche tecniche di cui hai bisogno sono qui dentro. Note operative in italiano; **i testi finali da pubblicare in pagina sono in inglese** e si trovano nelle sezioni "Copy" delimitate da `>` blockquote.

---

## 1. Cos'è JupiterOS

**Pitch corretto (usa questo, non altre versioni che hai potuto leggere in giro):**

JupiterOS è un'**applicazione desktop open-source basata su Tauri** che fornisce una **chat con Anthropic Claude** integrata nativamente con **server MCP (Model Context Protocol) locali e remoti**. L'utente gestisce i suoi MCP server visualmente dalla sidebar (start/stop/log/credentials) senza editare file di configurazione a mano. Ogni tool call di Claude è mostrata in una timeline trasparente con badge del server, durata e risultato inline.

**Non chiamarlo "AI Operating System".** È un chat client desktop con MCP integrati. Punto.

**3 selling point centrali:**

1. **Visual MCP management** — sidebar con tutti i server (locali stdio + remoti SSE/HTTP), status live, start/stop/log/credenziali. Niente `.mcp.json` da editare.
2. **Transparent tool timeline** — vedi ogni tool call inline: server, status, durata, risultato. Token e cost counter in real-time.
3. **No token wall** — slash command `/compact` comprime il contesto e continua la sessione. Multi-session, slash commands stile Claude Code (`/clear`, `/model`, `/cost`, `/mcp`, `/plan`).

**MCP server preconfigurati out-of-the-box:**

- **Io** — Email (IMAP/SMTP, calendario CalDAV, ricerca semantica)
- **Europa** — Messaggi (Telegram, ricerca semantica)
- **Amalthea** — Grafici e diagrammi deterministici
- **DashOps** — CRM/Operations (remoto via SSE, esempio di custom MCP)

L'utente può aggiungere qualsiasi altro MCP server (stdio/SSE/HTTP) editando un file di config nella UI.

**Modello di licenza:** open core, AGPL-3.0 per GUI + Moon free, source-available per Moon premium.

**Privacy:** zero telemetria, dati locali, l'utente porta la propria Anthropic API key.

**Autore:** Edoardo Mancinelli (Bambu Holding S.r.l.s.). Indie dev italiano. JupiterOS è il suo daily-driver da ~1 mese.

---

## 2. Stack tecnico richiesto

| Aspetto | Scelta | Motivazione |
|---------|--------|-------------|
| Framework | **Astro 5** | Static-first, zero JS runtime di default, ideale per landing |
| Styling | **Tailwind CSS v3** | Coerente con design system esistente della GUI |
| Animazioni | **CSS native** + `@keyframes` | Nessuna dipendenza, leggero |
| Icone | **Lucide** (SVG inline o pacchetto `astro-icon`) | Coerente, leggero |
| Font | **Inter** da Google Fonts (con `font-display: swap`) | Clean, ottima leggibilità |
| Hosting | **Cloudflare Pages** | Free, CDN globale, deploy automatico da Git, SSL automatico |
| Dominio | `jupiteros.ai` (GoDaddy → DNS Cloudflare) | Già acquistato |

**Non usare:** React, Vue, Svelte, Next.js. La landing è una pagina statica. Astro genera HTML puro, niente bundle JS pesante.

**Non aggiungere:** analytics tracciante (Google Analytics, etc.), cookie banner, chat widget, popup, newsletter form con tracking.

**Aggiungere (opzionale):** Cloudflare Web Analytics (privacy-first, no cookie, no banner).

---

## 3. Struttura progetto attesa

```
landing/
├── astro.config.mjs
├── package.json
├── tailwind.config.mjs
├── tsconfig.json
├── public/
│   ├── favicon.svg
│   ├── favicon.ico
│   ├── apple-touch-icon.png       180x180
│   ├── og-image.png               1200x630
│   ├── edoardo.jpg                ~800x1000 (3:4 ritratto)
│   ├── logo.svg                   logo JupiterOS vettoriale
│   ├── logo.png                   fallback raster
│   ├── demo-chat.gif              hero GIF — chat + tool call
│   ├── demo-mcp.gif               MCP sidebar start/stop
│   └── demo-compact.gif           /compact in azione
└── src/
    ├── layouts/
    │   └── Base.astro             <html>, <head>, meta SEO, font Inter
    ├── pages/
    │   └── index.astro            pagina unica, assembla i componenti
    ├── components/
    │   ├── Navbar.astro
    │   ├── Hero.astro
    │   ├── Pillars.astro
    │   ├── DemoGallery.astro
    │   ├── PreconfiguredMcps.astro
    │   ├── BuiltBy.astro
    │   ├── OpenSourcePrivacy.astro
    │   ├── Download.astro
    │   └── Footer.astro
    └── styles/
        └── global.css             base Tailwind + animazioni custom
```

---

## 4. Design system

### Palette (dark only — brand mono **nero + arancione**)

L'identità visiva è **nero e arancione**. Niente blu, niente accenti pastello. Il verde resta solo come stato funzionale ("ACTIVE" badge per i server MCP).

```css
--bg:          #0a0a0a;   /* background pagina, nero quasi puro */
--surface:     #141414;   /* card, sezioni alternate */
--elevated:    #1f1f1f;   /* hover states */
--border:      #2a2a2a;   /* card borders, dividers — grigio neutro */
--text:        #fafafa;   /* body, headings — bianco caldo */
--muted:       #a3a3a3;   /* secondary text */
--primary:     #ff6b1a;   /* ARANCIONE brand — CTA, link, accent */
--primary-hi:  #ff8c42;   /* arancione chiaro — hover/focus */
--secondary:   #cc6600;   /* arancione scuro — accenti muted, eventuali badge */
--success:     #3fb950;   /* verde — solo per status ACTIVE (funzionale, non brand) */
```

**Regole d'uso del colore:**

- Sfondo pagina: **sempre `--bg` (#0a0a0a)**. No gradient pieno-pagina, no immagini di sfondo.
- Headings e body: **`--text`**. Mai colorati di arancione per intero (l'arancione è per CTA e accent, non per blocchi di testo).
- Link inline e CTA primario: **`--primary`** con hover `--primary-hi`.
- Bordi card e divider: **`--border`**.
- Gradient e glow (hero, hover, focus ring): **solo arancione semi-trasparente**, mai blu.
- Verde `--success`: usato solo per piccoli indicatori di stato funzionali (es. cerchio pulsante "server attivo" se viene mostrato nei demo screenshot). Non per CTA né per bordi di sezione.

### Tipografia

| Uso | Font | Size | Weight | Line-height |
|---|---|---|---|---|
| Hero H1 | Inter | `clamp(2.5rem, 5vw, 4.5rem)` | 700 | 1.1 |
| H2 sezioni | Inter | `clamp(1.75rem, 3vw, 2.5rem)` | 700 | 1.2 |
| H3 card | Inter | `1.25rem` | 600 | 1.3 |
| Body | Inter | `1rem` (16px) | 400 | 1.6 |
| Small / muted | Inter | `0.875rem` | 400 | 1.5 |

### Spacing

- Container max-width: `1200px`, padding orizzontale `1.5rem` mobile / `2rem` desktop
- Sezioni: padding verticale `5rem` desktop, `3rem` mobile
- Gap tra card in griglie: `1.5rem`

### Breakpoint

- Mobile: default
- Tablet: `md:` ≥768px
- Desktop: `lg:` ≥1024px

### Effetti

- Card hover: `box-shadow: 0 0 24px rgba(255, 107, 26, 0.18)`, `border-color: #ff6b1a`, transizione `200ms`
- Navbar on scroll: `backdrop-filter: blur(12px)`, `background: rgba(10, 10, 10, 0.8)`
- Hero CTA primario: background `#ff6b1a`, hover `#ff8c42`
- Hero CTA secondario: outline `1px solid #2a2a2a`, hover `border-color: #ff6b1a`
- Badge status "ACTIVE": cerchio verde `#3fb950` con `animation: pulse 2s infinite`
- Fade-in on scroll per sezioni (intersection observer + opacity transition)

---

## 5. Sezioni della pagina

Ordine dall'alto al basso. Per ogni sezione: wireframe ASCII + copy in inglese pronto da copiare.

### 5.1 Navbar (sticky top)

```
┌───────────────────────────────────────────────────────────────┐
│  [logo] JupiterOS    Features  Demo  About    [GitHub →]    │
└───────────────────────────────────────────────────────────────┘
```

- Sticky in alto, full-width, padding verticale `1rem`
- Background trasparente al top → `rgba(13,17,23,0.8)` + `backdrop-blur(12px)` su scroll
- Logo a sinistra (SVG 28px h) + wordmark "JupiterOS" (Inter 600 18px)
- Link al centro: anchor a `#features`, `#demo`, `#about`
- CTA destra: link a `https://github.com/jupiter-os/jupiteros` con freccia
- Mobile: hamburger con stessi link (slide-down panel)

**Copy:**

> JupiterOS · Features · Demo · About · GitHub →

---

### 5.2 Hero

```
┌───────────────────────────────────────────────────────────────┐
│                                                               │
│         Your Claude. Your MCP servers.                        │
│         One open-source desktop.                              │
│                                                               │
│   JupiterOS is a Tauri-based chat client for Anthropic        │
│   Claude with native MCP support — local (stdio) and          │
│   remote (SSE/HTTP) servers, managed visually, no JSON        │
│   editing required.                                           │
│                                                               │
│   [Download for Windows / Mac / Linux]  [Star on GitHub →]    │
│                                                               │
│         ┌─────────────────────────────────────────┐           │
│         │                                         │           │
│         │       demo-chat.gif (hero GIF)          │           │
│         │       max-width 960px, rounded-xl       │           │
│         │       shadow + 1px border #2a2a2a       │           │
│         │                                         │           │
│         └─────────────────────────────────────────┘           │
└───────────────────────────────────────────────────────────────┘
```

- Gradient radiale sottile dietro il testo: `radial-gradient(circle at 50% 30%, rgba(255,107,26,0.18), transparent 60%)`
- Headline su 2 righe, centered
- CTA primario blu (`#ff6b1a`) + CTA secondario outline
- GIF hero centrata sotto i CTA, con bordo `#2a2a2a` e ombra blu sottile

**Copy:**

> # Your Claude. Your MCP servers.<br>One open-source desktop.
>
> JupiterOS is a Tauri-based chat client for Anthropic Claude with native MCP support — local (stdio) and remote (SSE/HTTP) servers, managed visually, no JSON editing required.
>
> [Download for Windows / Mac / Linux] [Star on GitHub →]

---

### 5.3 Three Pillars (`#features`)

```
┌───────────────────────────────────────────────────────────────┐
│                  Three things JupiterOS does well             │
│                                                               │
│   ┌──────────────┐   ┌──────────────┐   ┌──────────────┐     │
│   │  [icon]      │   │  [icon]      │   │  [icon]      │     │
│   │ Visual MCP   │   │ Transparent  │   │  No token    │     │
│   │ management   │   │ tool timeline│   │  wall        │     │
│   │              │   │              │   │              │     │
│   │ body copy    │   │ body copy    │   │ body copy    │     │
│   └──────────────┘   └──────────────┘   └──────────────┘     │
└───────────────────────────────────────────────────────────────┘
```

- 3 card affiancate desktop, stack verticale mobile
- Card: padding `2rem`, border `1px solid #2a2a2a`, radius `0.75rem`, background `#141414`
- Icone Lucide: `Layers`, `Activity`, `RefreshCw` (top-left della card, size 32px, color `#ff6b1a`)
- H3 titolo, body in `#a3a3a3`

**Copy:**

> ## Three things JupiterOS does well
>
> ### Visual MCP management
> See every MCP server in your sidebar — local stdio processes or remote SSE/HTTP endpoints. Start, stop, restart, tail logs and edit credentials right from the UI. No more hand-editing `.mcp.json`.
>
> ### Transparent tool timeline
> Watch Claude work in real time. Every tool call appears inline with the server it hits, a live status, elapsed time and the actual result. Token usage and cost are tracked per turn.
>
> ### No token wall
> Hit the context limit? Type `/compact` and JupiterOS summarises the conversation so far, hands it to a fresh session, and you keep going. Multi-session, slash commands, persistent history.

---

### 5.4 Demo Gallery (`#demo`)

```
┌───────────────────────────────────────────────────────────────┐
│                       See it in action                        │
│                                                               │
│   ┌─────────────────┐   ┌─────────────────┐   ┌─────────────┐ │
│   │ demo-chat.gif   │   │ demo-mcp.gif    │   │ demo-compact│ │
│   │                 │   │                 │   │ .gif        │ │
│   └─────────────────┘   └─────────────────┘   └─────────────┘ │
│   Chat with tools       Manage your MCPs       Beat the      │
│                                                  token wall   │
└───────────────────────────────────────────────────────────────┘
```

- Griglia 3 colonne desktop, 1 colonna mobile, gap `1.5rem`
- Ogni GIF in card con border `#2a2a2a`, radius `0.75rem`, overflow hidden
- Caption sotto ogni GIF in `#a3a3a3`, centered

**Copy:**

> ## See it in action
>
> **Chat with tools** — Ask anything. Claude calls the right MCP tool, results stream back inline.
>
> **Manage your MCPs** — Start, stop and inspect every server from the sidebar.
>
> **Beat the token wall** — `/compact` compresses your context and continues the conversation.

---

### 5.5 Pre-configured MCPs

```
┌───────────────────────────────────────────────────────────────┐
│            Comes pre-configured with useful MCPs              │
│                                                               │
│   ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐         │
│   │ Io      │  │ Europa  │  │Amalthea │  │DashOps  │         │
│   │ Email   │  │ Messaging│  │ Charts  │  │ CRM     │         │
│   │ IMAP/SMTP│ │ Telegram │  │16 types │  │ remote  │         │
│   └─────────┘  └─────────┘  └─────────┘  └─────────┘         │
│                                                               │
│   Bring your own — any stdio, SSE or HTTP MCP server works.   │
└───────────────────────────────────────────────────────────────┘
```

- Griglia 4 colonne desktop (2x2 tablet, 1 colonna mobile)
- Card piccole, padding `1.25rem`, border `1px solid #2a2a2a`
- Icona Lucide in alto a sinistra di ogni card: `Mail`, `MessageCircle`, `BarChart3`, `Briefcase`
- Sotto la griglia, riga centered con sfondo `#141414` e padding `1rem`, copy "Bring your own"

**Copy:**

> ## Comes pre-configured with useful MCPs
>
> **Io** — Email (IMAP/SMTP, CalDAV calendar, semantic search)
>
> **Europa** — Messaging (Telegram, semantic search)
>
> **Amalthea** — Charts and diagrams (16 deterministic types)
>
> **DashOps** — CRM and operations (example of a remote SSE MCP)
>
> Bring your own — any stdio, SSE or HTTP MCP server works out of the box.

---

### 5.6 Built by Edoardo Mancinelli (`#about`)

```
┌───────────────────────────────────────────────────────────────┐
│                                                               │
│   ┌──────────────┐    Built by Edoardo Mancinelli             │
│   │              │                                            │
│   │ edoardo.jpg  │    [bio text here, 3-4 sentences]          │
│   │ 3:4 ratio    │                                            │
│   │              │    [LinkedIn] [GitHub] [X] [YouTube]       │
│   │              │                                            │
│   └──────────────┘                                            │
└───────────────────────────────────────────────────────────────┘
```

- 2 colonne desktop (foto sinistra 1/3, testo destra 2/3), stack verticale mobile (foto sopra centrata, max-width 240px)
- Foto: `edoardo.jpg`, ratio 3:4, radius `0.75rem`, border `1px solid #2a2a2a`
- H2 "Built by Edoardo Mancinelli"
- Bio in 3-4 frasi (vedi sotto per template; **lasciare il testo definitivo da scegliere all'utente**)
- Riga icone social: LinkedIn, GitHub, X, YouTube (icone Lucide, size 24px, color `#a3a3a3`, hover `#ff6b1a`)

**Copy (template bio — propone questa, l'utente la rifinirà):**

> ## Built by Edoardo Mancinelli
>
> Indie developer based in Italy. I built JupiterOS because I wanted a Claude chat client that treats MCP servers as first-class citizens — visible, manageable, transparent. I use it every day to run my own email, messaging and operations through Claude. Now it's open source.
>
> [LinkedIn] [GitHub] [X] [YouTube]

**Placeholder iniziale per i link social** (`href="#"` con `aria-label`):

- LinkedIn — sostituire con URL profilo Edoardo
- GitHub — sostituire con URL profilo GitHub
- X — placeholder fino a quando l'account non è aperto
- YouTube — placeholder fino a quando il canale non è aperto

---

### 5.7 Open Source + Privacy

```
┌───────────────────────────────────────────────────────────────┐
│                                                               │
│   ┌────────────────────────┐   ┌────────────────────────┐    │
│   │  OPEN SOURCE           │   │  PRIVACY BY DESIGN     │    │
│   │  AGPL-3.0              │   │                        │    │
│   │                        │   │  Zero telemetry        │    │
│   │  Free Moons:           │   │  Data stays local      │    │
│   │  Io, Europa, Amalthea  │   │  Your Anthropic key    │    │
│   │                        │   │  Local embeddings      │    │
│   │  Premium Moons:        │   │  Local vector DB       │    │
│   │  source-available      │   │                        │    │
│   └────────────────────────┘   └────────────────────────┘    │
└───────────────────────────────────────────────────────────────┘
```

- 2 colonne uguali desktop, stack mobile
- Colonna sinistra: bordo `#3fb950` (verde), titolo "Open Source" — il verde qui è l'unica eccezione "non-arancione" e simboleggia "open / libero"
- Colonna destra: bordo `#ff6b1a` (arancione brand), titolo "Privacy by Design"
- Padding card `2rem`, radius `0.75rem`

**Copy:**

> ## Open and private
>
> ### Open Source — AGPL-3.0
> The desktop app and core Moons (Io, Europa, Amalthea) are AGPL-3.0. Premium Moons with advanced ML capabilities (browser automation, predictions, OCR) are source-available under a commercial license.
>
> ### Privacy by Design
> Zero telemetry. Your emails, messages and conversations stay on your machine. Embeddings run locally via ONNX. The vector store (Qdrant) runs locally. You bring your own Anthropic API key — the only outbound traffic is to `api.anthropic.com`.

---

### 5.8 Download

```
┌───────────────────────────────────────────────────────────────┐
│                       Download JupiterOS                      │
│                                                               │
│   ┌────────────┐    ┌────────────┐    ┌────────────┐         │
│   │ Windows    │    │  macOS     │    │  Linux     │         │
│   │ .msi       │    │  .dmg      │    │  .AppImage │         │
│   │            │    │            │    │            │         │
│   │ Download   │    │ Download   │    │ Download   │         │
│   └────────────┘    └────────────┘    └────────────┘         │
│                                                               │
│              Or build from source on GitHub →                 │
└───────────────────────────────────────────────────────────────┘
```

- 3 card affiancate desktop, stack mobile
- Ogni card: icona OS (Lucide o SVG custom), nome OS, formato installer, bottone "Download"
- I bottoni puntano a GitHub Releases URL — al momento del primo deploy, **lascia `href="#"` con classe disabled** e tooltip "Available with v0.1.0" finché il primo release non è pubblicato
- Sotto le 3 card, una riga centered con link "Or build from source on GitHub →" → `https://github.com/jupiter-os/jupiteros`

**Copy:**

> ## Download JupiterOS
>
> **Windows** · .msi installer
>
> **macOS** · .dmg
>
> **Linux** · .AppImage
>
> Or [build from source on GitHub →](https://github.com/jupiter-os/jupiteros)

---

### 5.9 Footer

```
┌───────────────────────────────────────────────────────────────┐
│  JupiterOS                          [GitHub] [X] [YouTube]    │
│  Open-source desktop chat for                                 │
│  Claude + MCP servers                                         │
│                                                               │
│  © 2026 Bambu Holding S.r.l.s. · Built in Italy               │
└───────────────────────────────────────────────────────────────┘
```

- Background `#141414`, padding verticale `3rem`
- Wordmark "JupiterOS" + tagline una riga in alto a sinistra
- Icone social in alto a destra (GitHub, X, YouTube)
- Riga copyright in basso, full-width, divider sopra (`1px solid #2a2a2a`)

**Copy:**

> JupiterOS — Open-source desktop chat for Claude + MCP servers
>
> [GitHub] [X] [YouTube]
>
> © 2026 Bambu Holding S.r.l.s. — Built in Italy

---

## 6. Asset richiesti

Tutti in `landing/public/`. Quelli marcati **PROVIDED** li mette l'utente, gli altri li generi tu.

| File | Dimensione | Note | Stato |
|---|---|---|---|
| `favicon.svg` | scalabile | Logo Jupiter minimale | Da generare (estrarre da `LOGO.png` esistente) |
| `favicon.ico` | 32×32 + 16×16 | Fallback browser legacy | Da generare |
| `apple-touch-icon.png` | 180×180 | iOS bookmark | Da generare |
| `og-image.png` | 1200×630 | Open Graph social share | Da generare (hero image + tagline sovrapposta) |
| `edoardo.jpg` | ~800×1000 (3:4) | Ritratto Edoardo, sfondo neutro | **PROVIDED** |
| `logo.svg` | scalabile | Logo wordmark per navbar | Da generare/estrarre |
| `logo.png` | 512×512 fallback | Raster | Da generare |
| `demo-chat.gif` | ~960×600, 2-3 MB | GIF chat con tool call live | **PROVIDED** (registrata da utente, vedi sez. 8) |
| `demo-mcp.gif` | ~960×600, 2-3 MB | GIF MCP sidebar start/stop | **PROVIDED** |
| `demo-compact.gif` | ~960×600, 2-3 MB | GIF `/compact` in azione | **PROVIDED** |

**Placeholder fino a quando gli asset PROVIDED arrivano:**

- `edoardo.jpg` → quadrato grigio `#1f1f1f` con iniziali "EM" centered in `#fafafa`
- `demo-*.gif` → rettangolo `#141414` con border `#2a2a2a` e testo "Demo coming soon" centered in `#a3a3a3`

---

## 7. SEO / meta tags

In `Base.astro`, dentro `<head>`:

```html
<title>JupiterOS — Open-source desktop chat for Claude + MCP servers</title>
<meta name="description" content="JupiterOS is a Tauri-based open-source chat client for Anthropic Claude with native MCP support. Manage local and remote MCP servers visually. Transparent tool timeline. AGPL-3.0.">
<meta name="theme-color" content="#0a0a0a">

<!-- Open Graph -->
<meta property="og:type" content="website">
<meta property="og:url" content="https://jupiteros.ai">
<meta property="og:title" content="JupiterOS — Open-source desktop chat for Claude + MCP servers">
<meta property="og:description" content="Tauri-based chat client for Anthropic Claude with native MCP support. Visual MCP management, transparent tool timeline, no token wall.">
<meta property="og:image" content="https://jupiteros.ai/og-image.png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">

<!-- Twitter -->
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="JupiterOS — Open-source desktop chat for Claude + MCP servers">
<meta name="twitter:description" content="Visual MCP management, transparent tool timeline, no token wall.">
<meta name="twitter:image" content="https://jupiteros.ai/og-image.png">

<!-- Favicon -->
<link rel="icon" type="image/svg+xml" href="/favicon.svg">
<link rel="icon" type="image/x-icon" href="/favicon.ico">
<link rel="apple-touch-icon" sizes="180x180" href="/apple-touch-icon.png">

<!-- Font Inter -->
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&display=swap" rel="stylesheet">
```

Astro genera automaticamente `robots.txt` e `sitemap.xml` se aggiungi `@astrojs/sitemap` integration.

---

## 8. Come registrare le 3 GIF (istruzioni per l'utente — incluse qui per autosufficienza)

**Tool consigliato (Windows, gratis):** [ScreenToGif](https://www.screentogif.com/)

**Settings consigliati:**
- Recording area: 960×600 (centrata sulla finestra JupiterOS)
- FPS: 15 (buon compromesso fluidità/peso)
- Durata: 6-10 secondi ciascuna
- Output: ottimizza per <3 MB con riduzione palette colori a 128

**Cosa registrare in ciascuna:**

1. **demo-chat.gif** — Apri una chat, scrivi una query reale (es. "list my last 5 emails"), mostra il tool call che parte nella timeline, il risultato che arriva inline, il token counter che si aggiorna.

2. **demo-mcp.gif** — Sidebar visibile, mostra status verde di 3-4 Moon, click su uno (es. Io), si apre il ServicePanel laterale, mostra i log che scorrono in tempo reale, poi click Stop → status diventa rosso, click Start → torna verde.

3. **demo-compact.gif** — Chat con qualche messaggio già presente, digita `/compact`, vedi il messaggio di conferma del summary, e una nuova query che continua naturalmente la conversazione.

Salva le 3 GIF in `landing/public/`.

---

## 9. Performance & accessibility targets

- **Lighthouse Performance**: ≥ 95
- **Lighthouse Accessibility**: 100
- **Lighthouse SEO**: 100
- **Lighthouse Best Practices**: ≥ 95
- **CLS** (Cumulative Layout Shift): < 0.05 (riserva spazio per le GIF con aspect-ratio o width/height espliciti)
- **LCP** (Largest Contentful Paint): < 1.5s
- **TBT** (Total Blocking Time): < 100ms (Astro genera zero JS di default, dovrebbe essere ~0)

**Accessibility:**

- Contrast ratio AAA su tutto il body text (`#fafafa` su `#0a0a0a` ≈ 20:1, ok). Per l'arancione `#ff6b1a` su `#0a0a0a` il ratio è ~7.8:1 (AAA per large text, AA per body) — usare solo per CTA, link e heading accent, non per long-form paragraph.
- Tutti i link e bottoni con `aria-label` dove l'icona è da sola (es. social footer)
- Skip-to-content link nascosto, visibile su focus tastiera
- Tutte le GIF con `alt` descrittivo
- Niente animazioni con `prefers-reduced-motion: reduce` rispettato (disabilita pulse, fade-in, hover glow)

---

## 10. Deploy

### Cloudflare Pages (consigliato)

1. Push del codice landing su GitHub (repo dedicato `jupiter-os/landing` o sottocartella del monorepo)
2. Cloudflare dashboard → Pages → "Create a project" → "Connect to Git"
3. Seleziona il repo, branch `main`
4. Build settings:
   - Framework preset: **Astro**
   - Build command: `npm run build`
   - Build output directory: `dist`
   - Root directory: `landing/` (se sottocartella del monorepo)
5. Deploy
6. Custom domain: in Pages → Custom domains → Add `jupiteros.ai` e `www.jupiteros.ai`
7. Cloudflare guida i passi per puntare il DNS: cambia i nameserver su GoDaddy con quelli che Cloudflare ti fornisce (es. `xyz.ns.cloudflare.com`, `abc.ns.cloudflare.com`). Propagazione 1-24 ore.
8. SSL: automatico, Cloudflare gestisce certificato Let's Encrypt
9. Redirect www → apex: in Cloudflare Rules → Page Rules → `www.jupiteros.ai/*` → 301 redirect a `https://jupiteros.ai/$1`

### Cloudflare Web Analytics (opzionale, privacy-first)

Dashboard CF → Analytics → Web Analytics → Add site `jupiteros.ai` → genera snippet JS minimale (no cookie, no consent banner) → incollare in `Base.astro` prima di `</body>`.

---

## 11. Non-goals (cosa NON fare)

- **Niente framework JS frontend** (React, Vue, Svelte): la landing è una pagina statica, Astro fa tutto da solo
- **Niente Google Analytics o tracker** terzi: viola il pitch privacy-first
- **Niente cookie banner**: zero cookie = zero banner necessario
- **Niente popup, exit-intent, scroll-jacking, autoplay video**: comportamenti aggressivi che bruciano la conversione su pubblico tecnico
- **Niente "Sign up for newsletter"** in hero: ha senso solo se hai backend per gestirla, e per ora non c'è
- **Niente tema chiaro / dark mode toggle**: dark-only coerente con la GUI
- **Niente i18n / multilingua**: solo inglese (decisione utente)
- **Niente sezione "Pricing"**: la versione free è AGPL, le Moon premium non sono ancora in vendita, mettere prezzi adesso confonde
- **Niente loghi clienti / testimonials fake**: il prodotto è appena open source, non inventare social proof

---

## 12. Checklist pre-launch

- [ ] Tutti i testi in inglese corretto (madrelingua sweep)
- [ ] Link GitHub punta al repo pubblico effettivo (non a `jupiter-os/jupiteros-OLD`)
- [ ] Link Download disabilitati con tooltip finché non c'è v0.1.0
- [ ] Foto Edoardo in `public/edoardo.jpg`
- [ ] 3 GIF demo in `public/demo-*.gif` con dimensioni ottimizzate
- [ ] OG image generata e testata su [OpenGraph debugger](https://www.opengraph.xyz/)
- [ ] Favicon visibile in tab browser
- [ ] Responsive testato su 375px, 768px, 1280px, 1920px
- [ ] Lighthouse audit eseguito su tutte le sezioni
- [ ] Test su Chrome, Safari, Firefox, Edge
- [ ] `prefers-reduced-motion` rispettato
- [ ] Tutti i `href="#"` placeholder identificati come TODO o sostituiti con link reali
- [ ] DNS jupiteros.ai propagato a Cloudflare
- [ ] HTTPS attivo, redirect www → apex funzionante

---

## 13. Riferimenti repository

- Repo attuale (privato, "OLD"): `https://github.com/jupiter-os/jupiteros-OLD`
- Repo target pubblico (al momento del lancio): `https://github.com/jupiter-os/jupiteros`
- Codice da cui è stato estratto il positioning: `jupiteros/src/App.tsx`, `jupiteros/src/hooks/useChat.ts`, `jupiteros/sidecar/agent.mjs`, `jupiteros/src/components/chat/ToolTimeline.tsx`, `jupiteros/src/components/chat/ChatArea.tsx`, `jupiteros/src-tauri/src/config.rs`

---

**Fine del brief.** Se mancano informazioni o trovi ambiguità nei testi, segnalalo prima di procedere.
