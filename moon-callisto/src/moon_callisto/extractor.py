# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Edoardo Mancinelli

"""Core extraction logic for Moon Callisto.

Strategy: never download the full video. If the video has subtitles (manual or
auto), grab them — it's free and instant. Otherwise extract ONLY the audio and
transcribe it locally with faster-whisper (CPU, offline).

The deterministic, mechanical part lives here. Deciding whether a video is
"fluff" is left to the human who reads the resulting transcript.
"""

import os
import re
import glob
import html
import json
import shutil
import logging
import subprocess
import tempfile
from datetime import date
from dataclasses import dataclass, field
from pathlib import Path

logger = logging.getLogger(__name__)


def _yt_dlp_bin() -> str:
    """Locate the yt-dlp binary (PATH, env override, or common ~/.local/bin)."""
    override = os.getenv("YT_DLP_BIN")
    if override:
        return override
    found = shutil.which("yt-dlp")
    if found:
        return found
    fallback = os.path.expanduser("~/.local/bin/yt-dlp")
    if os.path.exists(fallback):
        return fallback
    return "yt-dlp"  # let it fail loudly with a clear message


def _run(cmd: list[str], timeout: int = 120, **kwargs) -> subprocess.CompletedProcess:
    logger.info("exec: %s", " ".join(cmd))
    try:
        return subprocess.run(
            cmd, capture_output=True, text=True, timeout=timeout, **kwargs
        )
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(cmd, 124, "", "timed out")


# ============================================================
# Metadata (pre-screening, no download)
# ============================================================


@dataclass
class VideoInfo:
    title: str = ""
    uploader: str = ""
    channel: str = ""
    duration: int = 0  # seconds
    view_count: int | None = None
    upload_date: str = ""  # YYYYMMDD
    webpage_url: str = ""
    has_manual_subs: bool = False
    has_auto_subs: bool = False
    sub_langs: list[str] = field(default_factory=list)
    chapters: list[str] = field(default_factory=list)

    @property
    def duration_str(self) -> str:
        s = int(self.duration or 0)
        h, rem = divmod(s, 3600)
        m, sec = divmod(rem, 60)
        return f"{h}:{m:02d}:{sec:02d}" if h else f"{m}:{sec:02d}"


def get_info(url: str) -> VideoInfo:
    """Fetch metadata only (yt-dlp -J --skip-download). No media downloaded."""
    proc = _run([_yt_dlp_bin(), "-J", "--skip-download", "--no-warnings", url])
    if proc.returncode != 0:
        raise RuntimeError(f"yt-dlp metadata failed: {proc.stderr.strip()[:500]}")
    data = json.loads(proc.stdout)

    subs = data.get("subtitles") or {}
    autosubs = data.get("automatic_captions") or {}
    chapters = [c.get("title", "") for c in (data.get("chapters") or []) if c.get("title")]

    return VideoInfo(
        title=data.get("title", ""),
        uploader=data.get("uploader", ""),
        channel=data.get("channel", "") or data.get("uploader", ""),
        duration=int(data.get("duration") or 0),
        view_count=data.get("view_count"),
        upload_date=data.get("upload_date", "") or "",
        webpage_url=data.get("webpage_url", url),
        has_manual_subs=bool(subs),
        has_auto_subs=bool(autosubs),
        sub_langs=sorted(set(list(subs.keys()) + list(autosubs.keys()))),
        chapters=chapters,
    )


# ============================================================
# Subtitle path (free, instant)
# ============================================================


def _parse_srt(path: str) -> str:
    """SRT/VTT → clean plain text, stripping indices, timestamps and tags.

    Also collapses the rolling-duplicate effect typical of YouTube auto-subs,
    where each cue repeats the tail of the previous one.
    """
    raw = Path(path).read_text(encoding="utf-8", errors="ignore")
    # Matches SRT ("00:00:01,000 -->") and VTT ("00:01.000 -->", hour optional).
    ts_re = re.compile(r"(?:\d{1,2}:)?\d{1,2}:\d{2}[.,]\d{1,3}\s*-->")
    tag_re = re.compile(r"<[^>]+>")
    header_re = re.compile(r"^(WEBVTT|Kind:|Language:|NOTE\b)", re.IGNORECASE)

    lines: list[str] = []
    for block in re.split(r"\n\s*\n", raw):
        for ln in block.splitlines():
            ln = ln.strip()
            if not ln:
                continue
            if ln.isdigit():
                continue
            if ts_re.search(ln):
                continue
            if header_re.match(ln):
                continue
            ln = tag_re.sub("", ln)
            ln = html.unescape(ln)
            ln = re.sub(r"\s+", " ", ln).strip()
            if ln:
                lines.append(ln)

    # Dedup: drop a line if identical to the last kept one, or fully contained
    # in it (handles the auto-sub rolling window).
    out: list[str] = []
    for ln in lines:
        if out and (ln == out[-1] or ln in out[-1]):
            continue
        out.append(ln)

    text = " ".join(out)
    return re.sub(r"\s+", " ", text).strip()


def fetch_subtitles(url: str, lang_pref: str, tmp: str) -> tuple[str, str] | None:
    """Try to download subtitles, ONE language at a time in preference order.

    Requesting a single language per call keeps the request count low and avoids
    YouTube's HTTP 429 rate-limiting (which would otherwise trigger long retry
    sleeps). The .vtt is parsed directly — no ffmpeg/srt conversion needed.

    Returns (text, lang) or None if no usable subtitles were found.
    """
    langs = [l.strip() for l in lang_pref.split(",") if l.strip()] or ["en"]
    for lang in langs:
        proc = _run([
            _yt_dlp_bin(),
            "--write-subs", "--write-auto-subs",
            "--sub-langs", lang,
            "--skip-download", "--no-warnings",
            "-o", os.path.join(tmp, f"s_{lang}.%(ext)s"),
            url,
        ], timeout=90)
        files = sorted(
            glob.glob(os.path.join(tmp, f"s_{lang}*.vtt"))
            + glob.glob(os.path.join(tmp, f"s_{lang}*.srt"))
        )
        if not files:
            logger.info("no subs for lang=%s (%s)", lang, proc.stderr.strip()[:200])
            continue
        text = _parse_srt(files[0])
        if text:
            return text, lang
    return None


# ============================================================
# Whisper path (audio only, local, offline)
# ============================================================

_WHISPER_MODEL_CACHE: dict[str, object] = {}


def _get_whisper(model_size: str):
    """Lazily load (and cache) a faster-whisper model. Imported lazily so the
    subtitle path works even without faster-whisper installed."""
    if model_size in _WHISPER_MODEL_CACHE:
        return _WHISPER_MODEL_CACHE[model_size]
    from faster_whisper import WhisperModel  # heavy import, deferred

    # int8 on CPU: fast and low memory on machines without a GPU.
    model = WhisperModel(model_size, device="cpu", compute_type="int8")
    _WHISPER_MODEL_CACHE[model_size] = model
    return model


def transcribe_audio(
    url: str, tmp: str, model_size: str, spoken_lang: str | None
) -> tuple[str, str]:
    """Extract audio with yt-dlp, transcribe with faster-whisper.

    Returns (text, detected_or_given_lang).
    """
    proc = _run([
        _yt_dlp_bin(),
        "-x", "--audio-format", "mp3",
        "--no-warnings",
        "-o", os.path.join(tmp, "audio.%(ext)s"),
        url,
    ], timeout=600)
    audio = os.path.join(tmp, "audio.mp3")
    if proc.returncode != 0 or not os.path.exists(audio):
        raise RuntimeError(f"audio extraction failed: {proc.stderr.strip()[:500]}")

    model = _get_whisper(model_size)
    segments, info = model.transcribe(
        audio, language=spoken_lang, vad_filter=True
    )
    text = " ".join(seg.text.strip() for seg in segments)
    text = re.sub(r"\s+", " ", text).strip()
    return text, getattr(info, "language", spoken_lang or "")


# ============================================================
# Orchestration
# ============================================================


def slugify(title: str) -> str:
    s = re.sub(r"\s+", "_", title.strip())
    s = re.sub(r"[^A-Za-z0-9_\-]", "", s)
    return s[:120] or "video"


@dataclass
class TranscriptResult:
    title: str
    url: str
    method: str  # "subtitles" or "whisper"
    lang: str
    text: str
    word_count: int
    info: VideoInfo


def extract(
    url: str,
    lang_pref: str = "it,en",
    model_size: str = "medium",
    spoken_lang: str | None = None,
    force_whisper: bool = False,
) -> TranscriptResult:
    """Full pipeline: metadata → subtitles (unless forced) → else whisper."""
    info = get_info(url)
    with tempfile.TemporaryDirectory() as tmp:
        text, lang, method = "", "", ""
        if not force_whisper:
            subs = fetch_subtitles(url, lang_pref, tmp)
            if subs:
                text, lang = subs
                method = "subtitles"
        if not text:
            text, lang = transcribe_audio(url, tmp, model_size, spoken_lang)
            method = "whisper"

    wc = len(text.split())
    return TranscriptResult(
        title=info.title or url,
        url=info.webpage_url or url,
        method=method,
        lang=lang,
        text=text,
        word_count=wc,
        info=info,
    )


def to_markdown(r: TranscriptResult) -> str:
    """Render the transcript as a KB-ready .md with YAML frontmatter."""
    wpm = ""
    if r.info.duration:
        wpm = f"{round(r.word_count / (r.info.duration / 60))}"
    fm = [
        "---",
        f'title: "{r.title.replace(chr(34), chr(39))}"',
        f"source: {r.url}",
        f"fetched: {date.today().isoformat()}",
        f"channel: {r.info.channel}",
        f"duration: {r.info.duration_str}",
        f"upload_date: {r.info.upload_date}",
        f"views: {r.info.view_count if r.info.view_count is not None else ''}",
        f"lang: {r.lang}",
        f"method: {r.method}",
        f"word_count: {r.word_count}",
        f"words_per_minute: {wpm}",
    ]
    if r.info.chapters:
        fm.append("chapters:")
        fm.extend(f"  - {c}" for c in r.info.chapters)
    fm.append("---")
    return "\n".join(fm) + "\n\n" + r.text + "\n"
