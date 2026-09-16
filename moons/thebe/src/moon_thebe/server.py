# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Edoardo Mancinelli

"""Moon Thebe — Branded report renderer for JupiterOS.

MCP server that turns Markdown (written by Claude) into a branded PDF, applying a
company "brand kit" (logo, identity, colors, fonts, letterhead) configured once.

Separation of concerns: Claude writes the SUBSTANCE (structure, analysis, citations).
Thebe is a PURE RENDERER — it never writes the report content, only formats it.
For charts, ask Moon Amalthea to render them and reference the image in the Markdown.
"""

import os
import json
import logging
from datetime import datetime
from pathlib import Path

from dotenv import load_dotenv
from mcp.server.fastmcp import FastMCP

from .renderer import render_report_pdf, list_brands, load_brand

load_dotenv()

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

_mcp_host = os.getenv("MCP_HOST", "127.0.0.1")
_mcp_port = int(os.getenv("MCP_PORT", "8900"))
_output_dir = Path(os.getenv(
    "THEBE_OUTPUT_DIR",
    os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(__file__))), "output"),
))

mcp = FastMCP("thebe", host=_mcp_host, port=_mcp_port)


def _ensure_output_dir() -> Path:
    _output_dir.mkdir(parents=True, exist_ok=True)
    return _output_dir


# ============================================================
# TOOLS
# ============================================================


@mcp.tool()
def list_brand_profiles() -> str:
    """List the available brand profiles (company brand kits) that reports can use."""
    brands = list_brands()
    if not brands:
        return "No brand profiles found. Add one under brands/<name>/brand.json."
    return "Available brand profiles:\n" + "\n".join(f"- {b}" for b in brands)


@mcp.tool()
def get_brand_profile(brand: str = "emotion") -> str:
    """Show a brand profile's settings (logo, colors, fonts, footer/fiscal data).

    Use this to verify the brand kit before rendering, or to see what still needs
    to be filled in (placeholders look like '__________').

    Args:
        brand: Brand profile name (folder under brands/). Defaults to "emotion".
    """
    try:
        cfg = load_brand(brand)
    except ValueError as e:
        return f"Error: {e}"
    cfg.pop("_dir", None)
    has_logo = "yes" if cfg.pop("_logo_uri", "") else "NO (no logo.png in the brand folder)"
    return f"Brand '{brand}' (logo present: {has_logo}):\n" + json.dumps(cfg, indent=2, ensure_ascii=False)


@mcp.tool()
def render_report(
    markdown_content: str,
    title: str,
    brand: str = "emotion",
    subtitle: str = "",
    filename: str = "",
) -> str:
    """Render a Markdown report to a branded PDF. Deterministic: same input = same output.

    Thebe applies the brand kit (logo, colors, fonts, letterhead, footer). You provide
    the already-written report content as Markdown — Thebe does NOT write content.

    Workflow: write the report body in Markdown (headings, tables, lists, **bold**),
    then call this. The user sees the parameters and can approve/correct before render.
    For charts, render them with Moon Amalthea first and embed the image in the Markdown.

    Args:
        markdown_content: The report body in Markdown (GitHub-flavored: tables, lists, code).
        title: Report title shown in the cover/letterhead block.
        brand: Brand profile to apply (call list_brand_profiles to see options). Default "emotion".
        subtitle: Optional subtitle under the title (e.g. client name, period).
        filename: Output filename without extension. Auto-generated if empty.
    """
    if not markdown_content.strip():
        return "Error: 'markdown_content' is empty — provide the report body as Markdown."

    out_dir = _ensure_output_dir()
    if not filename:
        ts = datetime.now().strftime("%Y%m%d_%H%M%S")
        slug = "".join(c if c.isalnum() else "_" for c in title.lower())[:40].strip("_") or "report"
        filename = f"{slug}_{ts}"
    out_path = out_dir / f"{filename}.pdf"

    try:
        render_report_pdf(markdown_content, title, brand, out_path, subtitle=subtitle)
    except ValueError as e:
        return f"Error: {e}"
    except Exception as e:
        logger.exception("Report render failed")
        return f"Error rendering report: {e}"

    size = out_path.stat().st_size if out_path.exists() else 0
    return (
        f"Report rendered successfully!\n"
        f"- Title: {title}\n"
        f"- Brand: {brand}\n"
        f"- File: {out_path}\n"
        f"- Size: {size:,} bytes\n\n"
        f"Open the PDF to view the branded report."
    )


def main():
    """Entry point for the MCP server."""
    logger.info(f"Moon Thebe starting on {_mcp_host}:{_mcp_port}")
    logger.info(f"Output directory: {_output_dir}")
    mcp.run(transport="sse")


if __name__ == "__main__":
    main()
