# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Edoardo Mancinelli

"""Moon Callisto — Video → clean transcript extractor for JupiterOS.

MCP server that turns a video URL into clean text, ready for a knowledge base.
It never downloads the full video: subtitles if available (free, instant),
otherwise audio-only + local faster-whisper transcription (offline, CPU).

Callisto does the mechanical part. Deciding whether a video is worth keeping
("fluff" or not) is left to the human who reads the transcript.
"""

import os
import logging
from pathlib import Path

from dotenv import load_dotenv
from mcp.server.fastmcp import FastMCP

from .extractor import extract, get_info, to_markdown, slugify

load_dotenv()

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

_mcp_host = os.getenv("MCP_HOST", "127.0.0.1")
_mcp_port = int(os.getenv("MCP_PORT", "8500"))
_output_dir = Path(os.getenv(
    "CALLISTO_OUTPUT_DIR",
    os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(__file__))), "output"),
))

mcp = FastMCP("callisto", host=_mcp_host, port=_mcp_port)


def _ensure_output_dir() -> Path:
    _output_dir.mkdir(parents=True, exist_ok=True)
    return _output_dir


# ============================================================
# TOOLS
# ============================================================


@mcp.tool()
def get_video_info(url: str) -> str:
    """Fetch a video's metadata WITHOUT downloading anything.

    Use this FIRST to pre-screen a video before transcribing — title, channel,
    duration, views, upload date, and whether subtitles exist. Lets you skip
    fluff videos before spending any transcription time.

    Args:
        url: The video URL (YouTube and most yt-dlp-supported sites).
    """
    try:
        info = get_info(url)
    except Exception as e:
        logger.exception("get_video_info failed")
        return f"Error: {e}"

    subs = []
    if info.has_manual_subs:
        subs.append("manuali")
    if info.has_auto_subs:
        subs.append("automatici")
    sub_str = ", ".join(subs) if subs else "nessuno"

    lines = [
        f"# {info.title}",
        f"- **Canale**: {info.channel}",
        f"- **Durata**: {info.duration_str}",
        f"- **Views**: {info.view_count if info.view_count is not None else 'n/d'}",
        f"- **Data**: {info.upload_date or 'n/d'}",
        f"- **Sottotitoli**: {sub_str}"
        + (f" ({', '.join(info.sub_langs[:12])})" if info.sub_langs else ""),
        f"- **URL**: {info.webpage_url}",
    ]
    if info.chapters:
        lines.append(f"- **Capitoli ({len(info.chapters)})**: "
                     + "; ".join(info.chapters[:15]))
    lines.append("")
    method = "sottotitoli (istantaneo)" if subs else "Whisper (audio, più lento)"
    lines.append(f"➜ La trascrizione userà: **{method}**.")
    return "\n".join(lines)


@mcp.tool()
def transcribe_video(
    url: str,
    lang_pref: str = "it,en",
    model: str = "medium",
    spoken_lang: str = "",
    force_whisper: bool = False,
) -> str:
    """Extract a clean transcript from a video and save it as a KB-ready .md file.

    Tries subtitles first (free, instant); if none exist, downloads ONLY the
    audio and transcribes it locally with faster-whisper (offline, CPU). The
    full video is never downloaded.

    After this returns, READ the saved .md file to decide whether the content is
    worth keeping for the knowledge base.

    Args:
        url: The video URL.
        lang_pref: Comma-separated subtitle languages to prefer (e.g. "it,en").
        model: Whisper model size — tiny | base | small | medium | large-v3.
               "medium" is a good CPU compromise; "small" is faster.
        spoken_lang: Force the spoken language for Whisper (e.g. "it", "en").
                     Empty = auto-detect.
        force_whisper: Skip subtitles and always transcribe the audio.
    """
    try:
        result = extract(
            url,
            lang_pref=lang_pref,
            model_size=model,
            spoken_lang=spoken_lang or None,
            force_whisper=force_whisper,
        )
    except Exception as e:
        logger.exception("transcribe_video failed")
        return f"Error: {e}"

    md = to_markdown(result)
    out_dir = _ensure_output_dir()
    out_path = out_dir / f"{slugify(result.title)}.md"
    out_path.write_text(md, encoding="utf-8")

    preview = result.text[:600] + ("…" if len(result.text) > 600 else "")
    return (
        f"Trascrizione completata.\n"
        f"- **Titolo**: {result.title}\n"
        f"- **Metodo**: {result.method} (lang: {result.lang or 'n/d'})\n"
        f"- **Parole**: {result.word_count:,}\n"
        f"- **Durata video**: {result.info.duration_str}\n"
        f"- **File**: {out_path}\n\n"
        f"**Anteprima:**\n{preview}\n\n"
        f"➜ Leggi il file completo con Read per valutare se tenerlo nella KB."
    )


@mcp.tool()
def list_transcripts() -> str:
    """List the transcripts already extracted into the output directory."""
    out_dir = _ensure_output_dir()
    files = sorted(out_dir.glob("*.md"))
    if not files:
        return "Nessuna trascrizione presente in output/."
    lines = [f"# Trascrizioni ({len(files)})"]
    for f in files:
        kb = f.stat().st_size / 1024
        lines.append(f"- {f.name} ({kb:.0f} KB) — {f}")
    return "\n".join(lines)


def main():
    """Entry point for the MCP server."""
    logger.info(f"Moon Callisto starting on {_mcp_host}:{_mcp_port}")
    logger.info(f"Output directory: {_output_dir}")
    mcp.run(transport="sse")


if __name__ == "__main__":
    main()
