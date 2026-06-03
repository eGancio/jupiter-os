# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Edoardo Mancinelli

"""Chart rendering engine for Moon Amalthea.

Converts structured data into standalone HTML files using:
- ECharts 5.x for data charts (bar, line, pie, sankey, treemap, heatmap, network, tree, radar, scatter)
- Mermaid.js for diagrams (flowchart, mindmap, er, sequence)
- Pure HTML/CSS for layouts (kanban, timeline)

Each chart type has:
1. A catalog entry (description + use case)
2. A data schema (expected structure + example)
3. A builder function (data → library config)
4. A template (HTML shell with embedded JS library)
"""

import json
from pathlib import Path
from typing import Optional

from jinja2 import Environment, FileSystemLoader

TEMPLATES_DIR = Path(__file__).parent / "templates"
_jinja_env = Environment(loader=FileSystemLoader(str(TEMPLATES_DIR)), autoescape=False)

# ============================================================
# CHART CATALOG — descriptions and use cases
# ============================================================

CHART_CATALOG = {
    "ECharts (data visualization)": {
        "bar": {
            "description": "Vertical or horizontal bar chart for comparing categories",
            "use_case": "revenue by quarter, task counts by status, comparisons",
        },
        "line": {
            "description": "Line chart for trends over time",
            "use_case": "sales trends, growth curves, time series",
        },
        "pie": {
            "description": "Pie/donut chart for proportions",
            "use_case": "market share, budget allocation, composition",
        },
        "scatter": {
            "description": "Scatter plot for correlation between two variables",
            "use_case": "price vs quality, performance vs cost, distributions",
        },
        "radar": {
            "description": "Radar/spider chart for multi-dimensional comparison",
            "use_case": "skill profiles, product comparison, SWOT analysis",
        },
        "sankey": {
            "description": "Sankey diagram for flows between stages",
            "use_case": "budget flows, user funnels, energy/resource allocation",
        },
        "treemap": {
            "description": "Treemap for hierarchical data with size",
            "use_case": "disk usage, org budget breakdown, portfolio allocation",
        },
        "heatmap": {
            "description": "Heatmap/matrix for intensity across two dimensions",
            "use_case": "correlation matrix, activity by day/hour, priority matrix",
        },
        "network": {
            "description": "Network/graph diagram for relationships between entities",
            "use_case": "social networks, system dependencies, entity relationships",
        },
        "tree": {
            "description": "Tree diagram for hierarchical structures",
            "use_case": "org charts, decision trees, taxonomy, file structures",
        },
    },
    "Mermaid (diagrams)": {
        "flowchart": {
            "description": "Flowchart for processes, decisions, and workflows",
            "use_case": "business processes, algorithms, decision logic, pipelines",
        },
        "mindmap": {
            "description": "Mind map for brainstorming and idea organization",
            "use_case": "brainstorming, concept mapping, topic exploration",
        },
        "er": {
            "description": "Entity-Relationship diagram for data models",
            "use_case": "database schema, data models, system entities",
        },
        "sequence": {
            "description": "Sequence diagram for interactions over time",
            "use_case": "API flows, protocol sequences, system interactions",
        },
    },
    "HTML (layouts)": {
        "kanban": {
            "description": "Kanban board with columns and cards",
            "use_case": "task boards, project status, workflow stages",
        },
        "timeline": {
            "description": "Vertical timeline for chronological events",
            "use_case": "project milestones, history, roadmap, event sequences",
        },
    },
}

# Flat lookup: chart_type -> engine
_ECHARTS_TYPES = set(CHART_CATALOG["ECharts (data visualization)"].keys())
_MERMAID_TYPES = set(CHART_CATALOG["Mermaid (diagrams)"].keys())
_HTML_TYPES = set(CHART_CATALOG["HTML (layouts)"].keys())
_ALL_TYPES = _ECHARTS_TYPES | _MERMAID_TYPES | _HTML_TYPES


# ============================================================
# DATA SCHEMAS — expected structure + examples
# ============================================================

_SCHEMAS = {
    # --- ECharts ---
    "bar": {
        "schema": {
            "categories": ["string — x-axis labels"],
            "series": [{"name": "string — series name", "values": ["number — one per category"]}],
        },
        "example": {
            "categories": ["Q1", "Q2", "Q3", "Q4"],
            "series": [
                {"name": "Revenue", "values": [120, 200, 150, 280]},
                {"name": "Costs", "values": [80, 120, 100, 160]},
            ],
        },
        "options": {"horizontal": "bool (default false)", "stack": "bool (default false)"},
    },
    "line": {
        "schema": {
            "categories": ["string — x-axis labels"],
            "series": [{"name": "string", "values": ["number"]}],
        },
        "example": {
            "categories": ["Jan", "Feb", "Mar", "Apr", "May"],
            "series": [{"name": "Users", "values": [100, 150, 180, 220, 310]}],
        },
        "options": {"smooth": "bool (default true)", "area": "bool (default false)"},
    },
    "pie": {
        "schema": {"items": [{"name": "string — slice label", "value": "number — slice size"}]},
        "example": {
            "items": [
                {"name": "Email", "value": 40},
                {"name": "Telegram", "value": 35},
                {"name": "WhatsApp", "value": 25},
            ]
        },
        "options": {"donut": "bool (default false) — hollow center"},
    },
    "scatter": {
        "schema": {"series": [{"name": "string", "data": [["number x", "number y"]]}]},
        "example": {
            "series": [
                {"name": "Products", "data": [[10, 8.04], [8, 6.95], [13, 7.58], [9, 8.81]]},
            ]
        },
        "options": {},
    },
    "radar": {
        "schema": {
            "indicators": [{"name": "string — axis label", "max": "number — axis max value"}],
            "series": [{"name": "string", "values": ["number — one per indicator"]}],
        },
        "example": {
            "indicators": [
                {"name": "Speed", "max": 100},
                {"name": "Reliability", "max": 100},
                {"name": "Cost", "max": 100},
                {"name": "UX", "max": 100},
            ],
            "series": [
                {"name": "Product A", "values": [80, 90, 60, 70]},
                {"name": "Product B", "values": [60, 70, 80, 95]},
            ],
        },
        "options": {},
    },
    "sankey": {
        "schema": {
            "nodes": [{"name": "string — unique node name"}],
            "links": [{"source": "string — from node", "target": "string — to node", "value": "number — flow size"}],
        },
        "example": {
            "nodes": [{"name": "Budget"}, {"name": "Engineering"}, {"name": "Marketing"}, {"name": "Sales"}, {"name": "R&D"}, {"name": "Ads"}],
            "links": [
                {"source": "Budget", "target": "Engineering", "value": 40},
                {"source": "Budget", "target": "Marketing", "value": 30},
                {"source": "Budget", "target": "Sales", "value": 20},
                {"source": "Engineering", "target": "R&D", "value": 35},
                {"source": "Marketing", "target": "Ads", "value": 25},
            ],
        },
        "options": {},
    },
    "treemap": {
        "schema": {
            "name": "string — root label",
            "children": [{"name": "string", "value": "number (leaf) or omit (branch)", "children": ["(recursive)"]}],
        },
        "example": {
            "name": "Portfolio",
            "children": [
                {
                    "name": "Tech",
                    "children": [
                        {"name": "AAPL", "value": 40},
                        {"name": "GOOGL", "value": 30},
                    ],
                },
                {
                    "name": "Finance",
                    "children": [
                        {"name": "JPM", "value": 20},
                        {"name": "GS", "value": 10},
                    ],
                },
            ],
        },
        "options": {},
    },
    "heatmap": {
        "schema": {
            "x_labels": ["string — column labels"],
            "y_labels": ["string — row labels"],
            "values": [["number — [x_index, y_index, value]"]],
        },
        "example": {
            "x_labels": ["Mon", "Tue", "Wed", "Thu", "Fri"],
            "y_labels": ["Morning", "Afternoon", "Evening"],
            "values": [
                [0, 0, 5], [1, 0, 7], [2, 0, 3], [3, 0, 8], [4, 0, 2],
                [0, 1, 9], [1, 1, 4], [2, 1, 6], [3, 1, 5], [4, 1, 7],
                [0, 2, 3], [1, 2, 8], [2, 2, 2], [3, 2, 6], [4, 2, 9],
            ],
        },
        "options": {"min_color": "string (default '#304a6e')", "max_color": "string (default '#58a6ff')"},
    },
    "network": {
        "schema": {
            "nodes": [{"id": "string", "name": "string — display label", "group": "string (optional) — for coloring"}],
            "edges": [{"source": "string — from id", "target": "string — to id", "label": "string (optional)"}],
        },
        "example": {
            "nodes": [
                {"id": "1", "name": "Claude", "group": "AI"},
                {"id": "2", "name": "Moon Io", "group": "Moon"},
                {"id": "3", "name": "Moon Europa", "group": "Moon"},
                {"id": "4", "name": "Moon Amalthea", "group": "Moon"},
                {"id": "5", "name": "ChromaDB", "group": "DB"},
                {"id": "6", "name": "SQLite", "group": "DB"},
            ],
            "edges": [
                {"source": "1", "target": "2", "label": "email"},
                {"source": "1", "target": "3", "label": "CRM"},
                {"source": "1", "target": "4", "label": "charts"},
                {"source": "2", "target": "5"},
                {"source": "3", "target": "6"},
            ],
        },
        "options": {"layout": "string — 'force' (default) or 'circular'"},
    },
    "tree": {
        "schema": {
            "name": "string — root label",
            "children": [{"name": "string", "children": ["(recursive, optional)"]}],
        },
        "example": {
            "name": "CEO",
            "children": [
                {
                    "name": "CTO",
                    "children": [{"name": "Dev Lead"}, {"name": "DevOps"}],
                },
                {
                    "name": "CFO",
                    "children": [{"name": "Accounting"}, {"name": "Finance"}],
                },
            ],
        },
        "options": {"layout": "string — 'orthogonal' (default) or 'radial'", "orient": "string — 'TB' (default), 'LR', 'BT', 'RL'"},
    },
    # --- Mermaid ---
    "flowchart": {
        "schema": {
            "nodes": [{"id": "string", "label": "string", "shape": "string — round|diamond|rect|stadium|hex (default rect)"}],
            "edges": [{"from": "string — node id", "to": "string — node id", "label": "string (optional)"}],
            "direction": "string — TD (top-down), LR (left-right), BT, RL (default TD)",
        },
        "example": {
            "direction": "TD",
            "nodes": [
                {"id": "A", "label": "Start", "shape": "stadium"},
                {"id": "B", "label": "Process Data", "shape": "rect"},
                {"id": "C", "label": "Valid?", "shape": "diamond"},
                {"id": "D", "label": "Save", "shape": "rect"},
                {"id": "E", "label": "Error", "shape": "rect"},
            ],
            "edges": [
                {"from": "A", "to": "B"},
                {"from": "B", "to": "C"},
                {"from": "C", "to": "D", "label": "Yes"},
                {"from": "C", "to": "E", "label": "No"},
            ],
        },
        "options": {},
    },
    "mindmap": {
        "schema": {
            "root": "string — central topic",
            "children": [{"label": "string", "children": ["(recursive, optional)"]}],
        },
        "example": {
            "root": "JupiterOS",
            "children": [
                {
                    "label": "Moons",
                    "children": [
                        {"label": "Io — Email"},
                        {"label": "Europa — CRM"},
                        {"label": "Amalthea — Charts"},
                    ],
                },
                {
                    "label": "GUI",
                    "children": [
                        {"label": "Chat"},
                        {"label": "Panels"},
                    ],
                },
                {"label": "ML"},
            ],
        },
        "options": {},
    },
    "er": {
        "schema": {
            "entities": [{"name": "string", "attrs": ["string — 'field_name type constraint'"]}],
            "relations": [
                {
                    "from": "string — entity name",
                    "to": "string — entity name",
                    "label": "string — relationship verb",
                    "from_card": "string — ||, |{, o{, o| (default ||)",
                    "to_card": "string — ||, }|, }o, o| (default }o)",
                }
            ],
        },
        "example": {
            "entities": [
                {"name": "User", "attrs": ["id int PK", "name string", "email string"]},
                {"name": "Order", "attrs": ["id int PK", "total float", "date date"]},
                {"name": "Product", "attrs": ["id int PK", "name string", "price float"]},
            ],
            "relations": [
                {"from": "User", "to": "Order", "label": "places", "from_card": "||", "to_card": "}o"},
                {"from": "Order", "to": "Product", "label": "contains", "from_card": "||", "to_card": "}o"},
            ],
        },
        "options": {},
    },
    "sequence": {
        "schema": {
            "actors": ["string — participant names in order"],
            "messages": [
                {
                    "from": "string — actor name",
                    "to": "string — actor name",
                    "label": "string — message text",
                    "type": "string — solid (default), dashed, async",
                }
            ],
        },
        "example": {
            "actors": ["Browser", "Server", "Database"],
            "messages": [
                {"from": "Browser", "to": "Server", "label": "GET /api/users", "type": "solid"},
                {"from": "Server", "to": "Database", "label": "SELECT * FROM users", "type": "solid"},
                {"from": "Database", "to": "Server", "label": "rows[]", "type": "dashed"},
                {"from": "Server", "to": "Browser", "label": "200 JSON", "type": "dashed"},
            ],
        },
        "options": {},
    },
    # --- HTML native ---
    "kanban": {
        "schema": {
            "columns": [
                {
                    "title": "string — column header",
                    "color": "string (optional) — accent color",
                    "cards": [{"title": "string", "description": "string (optional)", "tag": "string (optional)", "color": "string (optional)"}],
                }
            ]
        },
        "example": {
            "columns": [
                {
                    "title": "To Do",
                    "color": "#58a6ff",
                    "cards": [
                        {"title": "Setup Tauri", "description": "Init project with React template", "tag": "core"},
                        {"title": "Chat component", "tag": "frontend"},
                    ],
                },
                {
                    "title": "In Progress",
                    "color": "#d29922",
                    "cards": [
                        {"title": "Moon Amalthea", "description": "Chart rendering engine", "tag": "moon"},
                    ],
                },
                {
                    "title": "Done",
                    "color": "#3fb950",
                    "cards": [
                        {"title": "Moon Io", "tag": "moon"},
                        {"title": "Moon Europa", "tag": "moon"},
                    ],
                },
            ]
        },
        "options": {},
    },
    "timeline": {
        "schema": {
            "events": [
                {
                    "date": "string — date/period label",
                    "title": "string — event title",
                    "description": "string (optional) — details",
                    "color": "string (optional) — accent color",
                }
            ]
        },
        "example": {
            "events": [
                {"date": "Week 1-2", "title": "Setup Tauri", "description": "App shell + service management"},
                {"date": "Week 3-5", "title": "Chat Claude", "description": "CLI integration + streaming"},
                {"date": "Week 6-7", "title": "Multi-tab", "description": "Multiple chat sessions"},
                {"date": "Week 8-9", "title": "Panels", "description": "Document viewer + schema viewer"},
                {"date": "Week 10-12", "title": "Moon Ganymede", "description": "Doc extraction + OCR"},
            ],
        },
        "options": {},
    },
}


def get_chart_schema(chart_type: str) -> Optional[dict]:
    """Return schema + example for a chart type, or None if unknown."""
    return _SCHEMAS.get(chart_type)


# ============================================================
# MAIN RENDER FUNCTION
# ============================================================


def render_chart(chart_type: str, data: dict, title: str = "", options: dict = None) -> str:
    """Render chart to HTML string. Raises ValueError for unknown types or bad data."""
    options = options or {}

    if chart_type not in _ALL_TYPES:
        raise ValueError(f"Unknown chart type '{chart_type}'. Available: {sorted(_ALL_TYPES)}")

    if chart_type in _ECHARTS_TYPES:
        return _render_echarts(chart_type, data, title, options)
    elif chart_type in _MERMAID_TYPES:
        return _render_mermaid(chart_type, data, title, options)
    else:
        return _render_html(chart_type, data, title, options)


# ============================================================
# ECHARTS RENDERER
# ============================================================

# Amalthea color palette
_PALETTE = ["#58a6ff", "#3fb950", "#d2a8ff", "#d29922", "#f85149", "#79c0ff", "#56d364", "#e3b341", "#ff7b72", "#a5d6ff"]


def _render_echarts(chart_type: str, data: dict, title: str, options: dict) -> str:
    builder = _ECHARTS_BUILDERS.get(chart_type)
    if not builder:
        raise ValueError(f"No ECharts builder for '{chart_type}'")

    echarts_option = builder(data, title, options)
    config_json = json.dumps(echarts_option, ensure_ascii=False)

    template = _jinja_env.get_template("echarts.html")
    return template.render(
        title=title or chart_type.capitalize(),
        chart_config=config_json,
    )


def _echarts_bar(data: dict, title: str, options: dict) -> dict:
    categories = data.get("categories", [])
    series_data = data.get("series", [])
    horizontal = options.get("horizontal", False)
    stack = options.get("stack", False)

    axis_type = {"type": "category", "data": categories, "axisLabel": {"color": "#8b949e"}, "axisLine": {"lineStyle": {"color": "#30363d"}}}
    value_axis = {"type": "value", "axisLabel": {"color": "#8b949e"}, "splitLine": {"lineStyle": {"color": "#21262d"}}}

    series = []
    for i, s in enumerate(series_data):
        item = {
            "name": s.get("name", f"Series {i+1}"),
            "type": "bar",
            "data": s.get("values", []),
            "itemStyle": {"color": _PALETTE[i % len(_PALETTE)]},
        }
        if stack:
            item["stack"] = "total"
        series.append(item)

    return {
        "backgroundColor": "transparent",
        "tooltip": {"trigger": "axis"},
        "legend": {"textStyle": {"color": "#c9d1d9"}, "top": 10},
        "grid": {"left": 60, "right": 30, "bottom": 40, "top": 60},
        "xAxis": value_axis if horizontal else axis_type,
        "yAxis": axis_type if horizontal else value_axis,
        "series": series,
    }


def _echarts_line(data: dict, title: str, options: dict) -> dict:
    categories = data.get("categories", [])
    series_data = data.get("series", [])
    smooth = options.get("smooth", True)
    area = options.get("area", False)

    series = []
    for i, s in enumerate(series_data):
        item = {
            "name": s.get("name", f"Series {i+1}"),
            "type": "line",
            "data": s.get("values", []),
            "smooth": smooth,
            "lineStyle": {"color": _PALETTE[i % len(_PALETTE)]},
            "itemStyle": {"color": _PALETTE[i % len(_PALETTE)]},
        }
        if area:
            item["areaStyle"] = {"opacity": 0.15}
        series.append(item)

    return {
        "backgroundColor": "transparent",
        "tooltip": {"trigger": "axis"},
        "legend": {"textStyle": {"color": "#c9d1d9"}, "top": 10},
        "grid": {"left": 60, "right": 30, "bottom": 40, "top": 60},
        "xAxis": {"type": "category", "data": categories, "axisLabel": {"color": "#8b949e"}, "axisLine": {"lineStyle": {"color": "#30363d"}}},
        "yAxis": {"type": "value", "axisLabel": {"color": "#8b949e"}, "splitLine": {"lineStyle": {"color": "#21262d"}}},
        "series": series,
    }


def _echarts_pie(data: dict, title: str, options: dict) -> dict:
    items = data.get("items", [])
    donut = options.get("donut", False)

    for i, item in enumerate(items):
        if "itemStyle" not in item:
            item["itemStyle"] = {"color": _PALETTE[i % len(_PALETTE)]}

    radius = ["45%", "70%"] if donut else [0, "70%"]

    return {
        "backgroundColor": "transparent",
        "tooltip": {"trigger": "item", "formatter": "{b}: {c} ({d}%)"},
        "legend": {"orient": "vertical", "right": 20, "top": "center", "textStyle": {"color": "#c9d1d9"}},
        "series": [
            {
                "type": "pie",
                "radius": radius,
                "center": ["45%", "55%"],
                "data": items,
                "label": {"color": "#c9d1d9"},
                "emphasis": {"itemStyle": {"shadowBlur": 10, "shadowColor": "rgba(0,0,0,0.5)"}},
            }
        ],
    }


def _echarts_scatter(data: dict, title: str, options: dict) -> dict:
    series_data = data.get("series", [])
    series = []
    for i, s in enumerate(series_data):
        series.append({
            "name": s.get("name", f"Series {i+1}"),
            "type": "scatter",
            "data": s.get("data", []),
            "itemStyle": {"color": _PALETTE[i % len(_PALETTE)]},
            "symbolSize": 12,
        })

    return {
        "backgroundColor": "transparent",
        "tooltip": {"trigger": "item"},
        "legend": {"textStyle": {"color": "#c9d1d9"}, "top": 10},
        "grid": {"left": 60, "right": 30, "bottom": 40, "top": 60},
        "xAxis": {"type": "value", "axisLabel": {"color": "#8b949e"}, "splitLine": {"lineStyle": {"color": "#21262d"}}},
        "yAxis": {"type": "value", "axisLabel": {"color": "#8b949e"}, "splitLine": {"lineStyle": {"color": "#21262d"}}},
        "series": series,
    }


def _echarts_radar(data: dict, title: str, options: dict) -> dict:
    indicators = data.get("indicators", [])
    series_data = data.get("series", [])

    series_items = []
    for i, s in enumerate(series_data):
        series_items.append({
            "name": s.get("name", f"Series {i+1}"),
            "value": s.get("values", []),
            "lineStyle": {"color": _PALETTE[i % len(_PALETTE)]},
            "itemStyle": {"color": _PALETTE[i % len(_PALETTE)]},
            "areaStyle": {"color": _PALETTE[i % len(_PALETTE)], "opacity": 0.1},
        })

    return {
        "backgroundColor": "transparent",
        "tooltip": {},
        "legend": {"textStyle": {"color": "#c9d1d9"}, "top": 10, "data": [s.get("name", "") for s in series_data]},
        "radar": {
            "indicator": [{"name": ind["name"], "max": ind.get("max", 100)} for ind in indicators],
            "axisName": {"color": "#c9d1d9"},
            "splitLine": {"lineStyle": {"color": "#21262d"}},
            "splitArea": {"areaStyle": {"color": ["transparent"]}},
            "axisLine": {"lineStyle": {"color": "#30363d"}},
        },
        "series": [{"type": "radar", "data": series_items}],
    }


def _echarts_sankey(data: dict, title: str, options: dict) -> dict:
    nodes = data.get("nodes", [])
    links = data.get("links", [])

    return {
        "backgroundColor": "transparent",
        "tooltip": {"trigger": "item", "triggerOn": "mousemove"},
        "series": [
            {
                "type": "sankey",
                "data": nodes,
                "links": links,
                "emphasis": {"focus": "adjacency"},
                "lineStyle": {"color": "gradient", "curveness": 0.5},
                "label": {"color": "#c9d1d9"},
                "itemStyle": {"borderWidth": 0},
            }
        ],
    }


def _echarts_treemap(data: dict, title: str, options: dict) -> dict:
    return {
        "backgroundColor": "transparent",
        "tooltip": {"formatter": "{b}: {c}"},
        "series": [
            {
                "type": "treemap",
                "data": [data],
                "roam": False,
                "label": {"show": True, "color": "#c9d1d9", "fontSize": 13},
                "breadcrumb": {"itemStyle": {"color": "#161b22", "textStyle": {"color": "#c9d1d9"}}},
                "levels": [
                    {"itemStyle": {"borderColor": "#0d1117", "borderWidth": 3, "gapWidth": 3}, "upperLabel": {"show": True, "color": "#c9d1d9"}},
                    {"itemStyle": {"borderColor": "#161b22", "borderWidth": 2, "gapWidth": 2}},
                    {"itemStyle": {"borderColor": "#21262d", "borderWidth": 1, "gapWidth": 1}},
                ],
            }
        ],
    }


def _echarts_heatmap(data: dict, title: str, options: dict) -> dict:
    x_labels = data.get("x_labels", [])
    y_labels = data.get("y_labels", [])
    values = data.get("values", [])

    all_vals = [v[2] for v in values if len(v) >= 3]
    min_val = min(all_vals) if all_vals else 0
    max_val = max(all_vals) if all_vals else 10

    return {
        "backgroundColor": "transparent",
        "tooltip": {"position": "top"},
        "grid": {"left": 80, "right": 60, "bottom": 40, "top": 40},
        "xAxis": {"type": "category", "data": x_labels, "axisLabel": {"color": "#8b949e"}, "axisLine": {"lineStyle": {"color": "#30363d"}}},
        "yAxis": {"type": "category", "data": y_labels, "axisLabel": {"color": "#8b949e"}, "axisLine": {"lineStyle": {"color": "#30363d"}}},
        "visualMap": {
            "min": min_val,
            "max": max_val,
            "calculable": True,
            "orient": "vertical",
            "right": 10,
            "top": "center",
            "inRange": {"color": [options.get("min_color", "#0d1117"), options.get("max_color", "#58a6ff")]},
            "textStyle": {"color": "#8b949e"},
        },
        "series": [
            {
                "type": "heatmap",
                "data": values,
                "label": {"show": True, "color": "#c9d1d9"},
                "emphasis": {"itemStyle": {"shadowBlur": 10, "shadowColor": "rgba(0,0,0,0.5)"}},
            }
        ],
    }


def _echarts_network(data: dict, title: str, options: dict) -> dict:
    nodes = data.get("nodes", [])
    edges = data.get("edges", [])
    layout = options.get("layout", "force")
    node_count = len(nodes)

    # Assign colors by group
    groups = list(dict.fromkeys(n.get("group", "") for n in nodes))
    group_colors = {g: _PALETTE[i % len(_PALETTE)] for i, g in enumerate(groups)}

    # Scale node size based on graph density — larger graphs get smaller nodes
    default_size = max(25, 50 - node_count)

    echarts_nodes = []
    for n in nodes:
        echarts_nodes.append({
            "id": str(n.get("id", n.get("name", ""))),
            "name": n.get("name", n.get("id", "")),
            "symbolSize": n.get("size", default_size),
            "category": groups.index(n.get("group", "")) if n.get("group") in groups else 0,
            "itemStyle": {"color": group_colors.get(n.get("group", ""), _PALETTE[0])},
        })

    echarts_edges = []
    for e in edges:
        edge = {"source": str(e["source"]), "target": str(e["target"])}
        if e.get("label"):
            edge["label"] = {"show": True, "formatter": e["label"], "color": "#8b949e", "fontSize": 11}
        echarts_edges.append(edge)

    categories = [{"name": g} for g in groups]

    # Scale force parameters based on node count to prevent overlap
    repulsion = options.get("repulsion", max(300, node_count * 60))
    edge_min = max(100, node_count * 8)
    edge_max = max(250, node_count * 18)
    gravity = 0.05 if node_count > 12 else 0.1

    series_config = {
        "type": "graph",
        "layout": layout,
        "data": echarts_nodes,
        "links": echarts_edges,
        "categories": categories,
        "roam": True,
        "label": {
            "show": True,
            "position": "bottom",
            "color": "#c9d1d9",
            "fontSize": 12,
            "distance": 5,
        },
        "lineStyle": {"color": "#30363d", "curveness": 0.2},
        "emphasis": {"focus": "adjacency", "lineStyle": {"width": 3}},
    }

    if layout == "force":
        series_config["force"] = {
            "repulsion": repulsion,
            "edgeLength": [edge_min, edge_max],
            "gravity": gravity,
            "friction": 0.6,
        }

    return {
        "backgroundColor": "transparent",
        "tooltip": {},
        "legend": [{"data": [c["name"] for c in categories], "textStyle": {"color": "#c9d1d9"}, "top": 10}],
        "series": [series_config],
    }


def _count_tree_nodes(node: dict) -> int:
    """Count total nodes in a tree structure."""
    count = 1
    for child in node.get("children", []):
        count += _count_tree_nodes(child)
    return count


def _max_label_length(node: dict) -> int:
    """Find the longest label in the tree."""
    longest = len(node.get("name", ""))
    for child in node.get("children", []):
        longest = max(longest, _max_label_length(child))
    return longest


def _echarts_tree(data: dict, title: str, options: dict) -> dict:
    layout = options.get("layout", "orthogonal")
    orient = options.get("orient", "TB")

    # Map TB/LR/etc to ECharts orient
    orient_map = {"TB": "TB", "BT": "BT", "LR": "LR", "RL": "RL"}
    ec_orient = orient_map.get(orient, "TB")

    # Dynamic sizing based on content
    node_count = _count_tree_nodes(data)
    max_label = _max_label_length(data)

    # Symbol width adapts to longest label (approx 7px per char + padding)
    symbol_w = max(80, min(200, max_label * 7 + 24))
    symbol_h = 30

    # Use "outside" labels for small symbols, "inside" for large ones
    label_inside = symbol_w >= max_label * 7

    # Vertical/horizontal: set generous margins so ECharts has room to space nodes
    is_vertical = ec_orient in ("TB", "BT")
    margins = {
        "top": 60,
        "bottom": 60,
        "left": 80 if is_vertical else 40,
        "right": 80 if is_vertical else 40,
    }

    return {
        "backgroundColor": "transparent",
        "tooltip": {"trigger": "item", "triggerOn": "mousemove"},
        "series": [
            {
                "type": "tree",
                "data": [data],
                "layout": layout,
                "orient": ec_orient,
                "symbol": "roundRect",
                "symbolSize": [symbol_w, symbol_h],
                "roam": True,
                "top": margins["top"],
                "bottom": margins["bottom"],
                "left": margins["left"],
                "right": margins["right"],
                "label": {
                    "position": "inside" if label_inside else "bottom",
                    "color": "#c9d1d9",
                    "fontSize": 12,
                    "backgroundColor": "#161b22",
                    "borderRadius": 4,
                    "padding": [4, 8],
                    "width": symbol_w - 16 if label_inside else None,
                    "overflow": "truncate",
                },
                "lineStyle": {"color": "#30363d", "width": 2},
                "leaves": {
                    "label": {
                        "position": "inside" if label_inside else "bottom",
                    }
                },
                "emphasis": {"focus": "descendant"},
                "expandAndCollapse": True,
                "initialTreeDepth": -1,
                "animationDuration": 550,
                "animationDurationUpdate": 750,
            }
        ],
    }


_ECHARTS_BUILDERS = {
    "bar": _echarts_bar,
    "line": _echarts_line,
    "pie": _echarts_pie,
    "scatter": _echarts_scatter,
    "radar": _echarts_radar,
    "sankey": _echarts_sankey,
    "treemap": _echarts_treemap,
    "heatmap": _echarts_heatmap,
    "network": _echarts_network,
    "tree": _echarts_tree,
}


# ============================================================
# MERMAID RENDERER
# ============================================================

def _render_mermaid(chart_type: str, data: dict, title: str, options: dict) -> str:
    builder = _MERMAID_BUILDERS.get(chart_type)
    if not builder:
        raise ValueError(f"No Mermaid builder for '{chart_type}'")

    mermaid_code = builder(data, options)

    template = _jinja_env.get_template("mermaid.html")
    return template.render(
        title=title or chart_type.capitalize(),
        mermaid_code=mermaid_code,
    )


def _mermaid_flowchart(data: dict, options: dict) -> str:
    direction = data.get("direction", "TD")
    nodes = data.get("nodes", [])
    edges = data.get("edges", [])

    lines = [f"flowchart {direction}"]

    shape_map = {
        "round": ("(", ")"),
        "stadium": ("([", "])"),
        "diamond": ("{", "}"),
        "rect": ("[", "]"),
        "hex": ("{{", "}}"),
        "circle": ("((", "))"),
        "parallelogram": ("[/", "/]"),
        "trapezoid": ("[/", "\\]"),
    }

    for node in nodes:
        nid = node["id"]
        label = node.get("label", nid)
        shape = node.get("shape", "rect")
        left, right = shape_map.get(shape, ("[", "]"))
        lines.append(f"    {nid}{left}\"{label}\"{right}")

    for edge in edges:
        src = edge["from"]
        tgt = edge["to"]
        label = edge.get("label", "")
        if label:
            lines.append(f"    {src} -->|\"{label}\"| {tgt}")
        else:
            lines.append(f"    {src} --> {tgt}")

    return "\n".join(lines)


def _mermaid_mindmap(data: dict, options: dict) -> str:
    lines = ["mindmap"]
    root = data.get("root", "Root")
    lines.append(f"  root(({root}))")

    def _add_children(children, depth=2):
        indent = "  " * depth
        for child in children:
            label = child.get("label", "")
            lines.append(f"{indent}{label}")
            if child.get("children"):
                _add_children(child["children"], depth + 1)

    _add_children(data.get("children", []))
    return "\n".join(lines)


def _mermaid_er(data: dict, options: dict) -> str:
    lines = ["erDiagram"]
    entities = data.get("entities", [])
    relations = data.get("relations", [])

    for entity in entities:
        name = entity["name"]
        lines.append(f"    {name} {{")
        for attr in entity.get("attrs", []):
            parts = attr.split()
            if len(parts) >= 2:
                lines.append(f"        {parts[1]} {parts[0]} {''.join(parts[2:])}")
            else:
                lines.append(f"        string {attr}")
        lines.append("    }")

    for rel in relations:
        from_card = rel.get("from_card", "||")
        to_card = rel.get("to_card", "}o")
        label = rel.get("label", "relates")
        lines.append(f"    {rel['from']} {from_card}--{to_card} {rel['to']} : \"{label}\"")

    return "\n".join(lines)


def _mermaid_sequence(data: dict, options: dict) -> str:
    lines = ["sequenceDiagram"]
    actors = data.get("actors", [])
    messages = data.get("messages", [])

    for actor in actors:
        lines.append(f"    participant {actor}")

    for msg in messages:
        src = msg["from"]
        tgt = msg["to"]
        label = msg.get("label", "")
        msg_type = msg.get("type", "solid")

        arrow_map = {"solid": "->>", "dashed": "-->>", "async": "-)"}
        arrow = arrow_map.get(msg_type, "->>")
        lines.append(f"    {src}{arrow}{tgt}: {label}")

    return "\n".join(lines)


_MERMAID_BUILDERS = {
    "flowchart": _mermaid_flowchart,
    "mindmap": _mermaid_mindmap,
    "er": _mermaid_er,
    "sequence": _mermaid_sequence,
}


# ============================================================
# HTML NATIVE RENDERER
# ============================================================

def _render_html(chart_type: str, data: dict, title: str, options: dict) -> str:
    template = _jinja_env.get_template("native.html")
    builder = _HTML_BUILDERS.get(chart_type)
    if not builder:
        raise ValueError(f"No HTML builder for '{chart_type}'")

    body_html, extra_css = builder(data, options)

    return template.render(
        title=title or chart_type.capitalize(),
        body_html=body_html,
        extra_css=extra_css,
    )


def _html_kanban(data: dict, options: dict) -> tuple[str, str]:
    columns = data.get("columns", [])

    css = """
.kanban { display: flex; gap: 20px; padding: 24px; overflow-x: auto; min-height: 80vh; align-items: flex-start; }
.kanban-col { background: #161b22; border-radius: 12px; min-width: 280px; max-width: 320px; flex-shrink: 0; }
.kanban-col-header { padding: 16px 20px; font-weight: 600; font-size: 14px; color: #c9d1d9; display: flex; align-items: center; gap: 10px; border-bottom: 2px solid var(--accent); }
.kanban-col-header .count { background: #21262d; color: #8b949e; border-radius: 10px; padding: 2px 8px; font-size: 12px; }
.kanban-cards { padding: 12px; display: flex; flex-direction: column; gap: 10px; }
.kanban-card { background: #0d1117; border: 1px solid #21262d; border-radius: 8px; padding: 14px; transition: border-color 0.2s; cursor: default; }
.kanban-card:hover { border-color: #30363d; }
.kanban-card h4 { margin: 0 0 6px 0; color: #c9d1d9; font-size: 14px; font-weight: 500; }
.kanban-card p { margin: 0; color: #8b949e; font-size: 12px; line-height: 1.5; }
.kanban-card .tag { display: inline-block; margin-top: 8px; padding: 2px 8px; border-radius: 10px; font-size: 11px; background: #21262d; color: #8b949e; }
"""

    parts = ['<div class="kanban">']
    for col in columns:
        accent = col.get("color", "#58a6ff")
        cards = col.get("cards", [])
        parts.append(f'<div class="kanban-col" style="--accent: {accent}">')
        parts.append(f'<div class="kanban-col-header"><span style="color:{accent}">●</span> {col["title"]} <span class="count">{len(cards)}</span></div>')
        parts.append('<div class="kanban-cards">')
        for card in cards:
            parts.append('<div class="kanban-card">')
            parts.append(f'<h4>{card["title"]}</h4>')
            if card.get("description"):
                parts.append(f'<p>{card["description"]}</p>')
            if card.get("tag"):
                color = card.get("color", "#8b949e")
                parts.append(f'<span class="tag" style="color:{color};border:1px solid {color}">{card["tag"]}</span>')
            parts.append('</div>')
        parts.append('</div></div>')
    parts.append('</div>')

    return "\n".join(parts), css


def _html_timeline(data: dict, options: dict) -> tuple[str, str]:
    events = data.get("events", [])

    css = """
.timeline { position: relative; max-width: 700px; margin: 40px auto; padding: 20px 0; }
.timeline::before { content: ''; position: absolute; left: 24px; top: 0; bottom: 0; width: 2px; background: #21262d; }
.tl-event { position: relative; padding-left: 60px; margin-bottom: 32px; }
.tl-dot { position: absolute; left: 17px; top: 4px; width: 16px; height: 16px; border-radius: 50%; border: 3px solid var(--accent, #58a6ff); background: #0d1117; z-index: 1; }
.tl-date { color: var(--accent, #58a6ff); font-size: 13px; font-weight: 600; margin-bottom: 4px; letter-spacing: 0.5px; }
.tl-title { color: #c9d1d9; font-size: 16px; font-weight: 600; margin-bottom: 4px; }
.tl-desc { color: #8b949e; font-size: 13px; line-height: 1.6; }
"""

    parts = ['<div class="timeline">']
    for event in events:
        accent = event.get("color", "#58a6ff")
        parts.append(f'<div class="tl-event" style="--accent:{accent}">')
        parts.append(f'<div class="tl-dot"></div>')
        parts.append(f'<div class="tl-date">{event.get("date", "")}</div>')
        parts.append(f'<div class="tl-title">{event.get("title", "")}</div>')
        if event.get("description"):
            parts.append(f'<div class="tl-desc">{event["description"]}</div>')
        parts.append('</div>')
    parts.append('</div>')

    return "\n".join(parts), css


_HTML_BUILDERS = {
    "kanban": _html_kanban,
    "timeline": _html_timeline,
}
