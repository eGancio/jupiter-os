"""Moon Amalthea — Deterministic chart & diagram renderer for JupiterOS.

MCP server that generates standalone HTML visualizations from structured data.
Claude structures the data, the user approves, Amalthea renders deterministically.
"""

import os
import json
import logging
from datetime import datetime
from pathlib import Path

from dotenv import load_dotenv
from mcp.server.fastmcp import FastMCP

from .renderer import render_chart, CHART_CATALOG, get_chart_schema

load_dotenv()

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

_mcp_host = os.getenv("MCP_HOST", "127.0.0.1")
_mcp_port = int(os.getenv("MCP_PORT", "8300"))
_output_dir = Path(os.getenv(
    "AMALTHEA_OUTPUT_DIR",
    os.path.join(os.path.dirname(os.path.dirname(os.path.dirname(__file__))), "output"),
))

mcp = FastMCP("amalthea", host=_mcp_host, port=_mcp_port)


def _ensure_output_dir() -> Path:
    _output_dir.mkdir(parents=True, exist_ok=True)
    return _output_dir


# ============================================================
# TOOLS
# ============================================================


@mcp.tool()
def list_chart_types() -> str:
    """List all available chart types with descriptions and use cases.

    Call this first to understand which chart type fits the user's request.
    Returns chart types grouped by rendering engine (ECharts, Mermaid, HTML).
    """
    lines = ["# Available Chart Types\n"]

    for engine, types in CHART_CATALOG.items():
        lines.append(f"## {engine}")
        for name, info in types.items():
            lines.append(f"- **{name}**: {info['description']}")
            lines.append(f"  Use for: {info['use_case']}")
        lines.append("")

    return "\n".join(lines)


@mcp.tool()
def get_data_schema(chart_type: str) -> str:
    """Get the expected data schema and example for a specific chart type.

    Call this BEFORE render_chart to understand the exact data format required.
    This prevents interpretation errors — you'll know exactly what structure to send.

    Args:
        chart_type: One of the types from list_chart_types (e.g. "flowchart", "bar", "mindmap")
    """
    schema = get_chart_schema(chart_type)
    if schema is None:
        return f"Error: unknown chart type '{chart_type}'. Call list_chart_types() to see available types."

    return (
        f"# Schema for '{chart_type}'\n\n"
        f"## Data structure\n```json\n{json.dumps(schema['schema'], indent=2, ensure_ascii=False)}\n```\n\n"
        f"## Example\n```json\n{json.dumps(schema['example'], indent=2, ensure_ascii=False)}\n```\n\n"
        f"## Options (optional)\n```json\n{json.dumps(schema.get('options', {}), indent=2, ensure_ascii=False)}\n```"
    )


@mcp.tool()
def render(
    chart_type: str,
    data: str,
    title: str = "",
    options: str = "{}",
    filename: str = "",
) -> str:
    """Render a chart to a standalone HTML file. Deterministic: same input = same output.

    IMPORTANT: Call get_data_schema first to verify the expected data format.
    The user can see the tool call parameters and approve/correct before rendering.

    Args:
        chart_type: Chart type (e.g. "flowchart", "bar", "mindmap", "network", "sankey")
        data: JSON string with chart data (structure depends on chart_type — use get_data_schema)
        title: Chart title displayed at the top
        options: JSON string with style options (theme, colors, layout). Optional.
        filename: Output filename (without extension). Auto-generated if empty.
    """
    try:
        data_dict = json.loads(data)
    except json.JSONDecodeError as e:
        return f"Error: invalid JSON in 'data' parameter: {e}"

    try:
        options_dict = json.loads(options)
    except json.JSONDecodeError as e:
        return f"Error: invalid JSON in 'options' parameter: {e}"

    try:
        html = render_chart(chart_type, data_dict, title, options_dict)
    except ValueError as e:
        return f"Error: {e}"
    except Exception as e:
        logger.exception("Render failed")
        return f"Error rendering chart: {e}"

    # Save to file
    out_dir = _ensure_output_dir()
    if not filename:
        ts = datetime.now().strftime("%Y%m%d_%H%M%S")
        filename = f"{chart_type}_{ts}"

    out_path = out_dir / f"{filename}.html"
    out_path.write_text(html, encoding="utf-8")

    return (
        f"Chart rendered successfully!\n"
        f"- Type: {chart_type}\n"
        f"- Title: {title or '(none)'}\n"
        f"- File: {out_path}\n"
        f"- Size: {len(html):,} bytes\n\n"
        f"Open the HTML file in a browser to view the chart."
    )


def main():
    """Entry point for the MCP server."""
    logger.info(f"Moon Amalthea starting on {_mcp_host}:{_mcp_port}")
    logger.info(f"Output directory: {_output_dir}")
    mcp.run(transport="sse")


if __name__ == "__main__":
    main()
