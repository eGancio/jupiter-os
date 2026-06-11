# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Edoardo Mancinelli

"""Report rendering engine for Moon Thebe.

Pipeline (deterministic): Markdown content + a brand profile -> themed HTML
(Jinja2 template injecting logo / colors / fonts / letterhead) -> PDF (WeasyPrint).

A "brand profile" is a folder under brands/<name>/ with a brand.json and a logo.
The brand kit (the constants of a company report — logo, company identity, colors,
fonts, footer) is configured ONCE and applied to every report.
"""

import os
import json
from datetime import datetime
from pathlib import Path
from typing import Optional

import markdown as md
from jinja2 import Environment, FileSystemLoader, select_autoescape
from weasyprint import HTML

BASE_DIR = Path(__file__).parent
TEMPLATES_DIR = BASE_DIR / "templates"
# Brand kits live in a writable DATA dir (set via THEBE_BRANDS_DIR so the GUI can
# configure them); falls back to the bundled defaults inside the package.
BRANDS_DIR = Path(os.getenv("THEBE_BRANDS_DIR") or (BASE_DIR / "brands"))

_jinja_env = Environment(
    loader=FileSystemLoader(str(TEMPLATES_DIR)),
    autoescape=select_autoescape(["html"]),
)

# Default brand kit, used to fill any field a profile leaves out.
_DEFAULT_BRAND = {
    "name": "Company",
    "tagline": "",
    "logo": "",
    "colors": {
        "primary": "#1a1a1a",
        "accent": "#2d7bbf",
        "text": "#1a1a1a",
        "muted": "#6b7280",
        "rule": "#e5e7eb",
    },
    "font": {"family": "Helvetica, Arial, sans-serif"},
    "footer": {
        "company": "",
        "vat": "",
        "address": "",
        "contacts": "",
        "confidentiality": "",
    },
}


def list_brands() -> list[str]:
    """Names of the available brand profiles (folders under brands/)."""
    if not BRANDS_DIR.exists():
        return []
    return sorted(
        p.name
        for p in BRANDS_DIR.iterdir()
        if p.is_dir() and (p / "brand.json").exists()
    )


def _deep_merge(base: dict, override: dict) -> dict:
    out = dict(base)
    for k, v in override.items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = _deep_merge(out[k], v)
        else:
            out[k] = v
    return out


def load_brand(name: str) -> dict:
    """Load a brand profile, merged over defaults. Raises ValueError if unknown."""
    brand_dir = BRANDS_DIR / name
    cfg_path = brand_dir / "brand.json"
    if not cfg_path.exists():
        available = ", ".join(list_brands()) or "(none)"
        raise ValueError(f"unknown brand '{name}'. Available: {available}")
    cfg = json.loads(cfg_path.read_text(encoding="utf-8"))
    merged = _deep_merge(_DEFAULT_BRAND, cfg)
    merged["_dir"] = str(brand_dir)
    # Resolve the logo to an absolute file:// URI if it exists.
    logo = merged.get("logo") or ""
    logo_path = (brand_dir / logo) if logo else None
    merged["_logo_uri"] = logo_path.resolve().as_uri() if (logo_path and logo_path.exists()) else ""
    return merged


def render_report_html(
    markdown_content: str,
    title: str,
    brand: dict,
    subtitle: str = "",
    date_str: Optional[str] = None,
) -> str:
    """Markdown + brand -> a full standalone HTML document (string)."""
    body_html = md.markdown(
        markdown_content,
        extensions=["extra", "tables", "fenced_code", "sane_lists", "toc"],
        output_format="html5",
    )
    template = _jinja_env.get_template("report.html")
    return template.render(
        title=title,
        subtitle=subtitle,
        date_str=date_str or datetime.now().strftime("%d/%m/%Y"),
        body=body_html,
        brand=brand,
    )


def render_report_pdf(
    markdown_content: str,
    title: str,
    brand_name: str,
    out_path: Path,
    subtitle: str = "",
    date_str: Optional[str] = None,
) -> Path:
    """Full pipeline -> writes a branded PDF to out_path, returns it."""
    brand = load_brand(brand_name)
    html = render_report_html(markdown_content, title, brand, subtitle, date_str)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    # base_url lets relative assets (logo, fonts) in the brand dir resolve.
    HTML(string=html, base_url=brand["_dir"]).write_pdf(str(out_path))
    return out_path
