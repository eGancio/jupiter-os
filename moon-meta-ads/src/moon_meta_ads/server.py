# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Edoardo Mancinelli

"""Moon Meta Ads — MCP server (stdio) for JupiterOS.

Talks DIRECTLY to the Meta Graph API (no third-party proxy) using the System
User token JupiterOS stores in the OS keyring (injected as META_ACCESS_TOKEN +
META_AD_ACCOUNT_ID). Same design as moon-google-ads: stdio transport, lazy
credential read so the MCP handshake is instant.

Tools:
- get_insights            — campaign/ad set KPIs (READ)
- list_campaigns          — campaigns with budget/status (READ)
- search_interests        — interest targeting research + audience size (READ)
- estimate_audience       — combined reach estimate for a targeting spec (READ)
- create_adset            — create an ad set (WRITE — gated by CLAUDE.md REGOLA #10,
                            created PAUSED; the agent must show a draft and get
                            explicit confirmation before calling this).
"""

import json
import os

import requests
from dotenv import load_dotenv
from mcp.server.fastmcp import FastMCP

load_dotenv()

mcp = FastMCP("meta-ads")

GRAPH = "https://graph.facebook.com/v23.0"


def _token() -> str:
    t = os.environ.get("META_ACCESS_TOKEN", "").strip()
    if not t:
        raise RuntimeError(
            "META_ACCESS_TOKEN mancante. Inserisci il token in JupiterOS "
            "(Service Detail → Meta Ads) e riavvia."
        )
    return t


def _acct() -> str:
    a = os.environ.get("META_AD_ACCOUNT_ID", "").strip()
    if not a:
        raise RuntimeError("META_AD_ACCOUNT_ID mancante (es. act_1234567890).")
    return a if a.startswith("act_") else f"act_{a}"


def _get(path: str, params: dict) -> dict:
    p = {**params, "access_token": _token()}
    r = requests.get(f"{GRAPH}/{path}", params=p, timeout=40)
    data = r.json()
    if isinstance(data, dict) and "error" in data:
        raise RuntimeError(f"Errore Meta: {data['error'].get('message', data['error'])}")
    return data


def _err(e: Exception) -> str:
    return f"Errore: {e}"


@mcp.tool()
def get_insights(date_preset: str = "last_30d", level: str = "campaign", limit: int = 25) -> str:
    """KPI/andamento delle inserzioni Meta dell'ad account.

    Args:
        date_preset: intervallo (today, yesterday, last_7d, last_30d, this_month, maximum…).
        level: account | campaign | adset | ad.
        limit: numero massimo di righe.
    Ritorna JSON con spesa, impression, click, CTR, CPC, reach, conversioni (actions).
    """
    try:
        fields = "campaign_name,adset_name,spend,impressions,clicks,ctr,cpc,reach,actions"
        data = _get(f"{_acct()}/insights",
                    {"level": level, "date_preset": date_preset, "fields": fields, "limit": limit})
        return json.dumps(data.get("data", []), ensure_ascii=False, indent=2)
    except Exception as e:  # noqa: BLE001
        return _err(e)


@mcp.tool()
def list_campaigns(limit: int = 50) -> str:
    """Elenca le campagne dell'ad account (nome, stato, obiettivo, budget)."""
    try:
        data = _get(f"{_acct()}/campaigns",
                    {"fields": "name,status,objective,daily_budget,lifetime_budget,start_time",
                     "limit": limit})
        return json.dumps(data.get("data", []), ensure_ascii=False, indent=2)
    except Exception as e:  # noqa: BLE001
        return _err(e)


@mcp.tool()
def search_interests(query: str, limit: int = 25) -> str:
    """Cerca INTERESSI di targeting Meta per un termine, con la STIMA del pubblico.

    Ritorna per ogni interesse: id, nome, dimensione pubblico (lower/upper), percorso
    tassonomico. Usa gli id restituiti in `estimate_audience` o per creare un ad set.
    """
    try:
        data = _get("search", {"type": "adinterest", "q": query, "limit": limit})
        out = [
            {
                "id": i.get("id"),
                "name": i.get("name"),
                "audience_lower": i.get("audience_size_lower_bound"),
                "audience_upper": i.get("audience_size_upper_bound"),
                "path": i.get("path"),
            }
            for i in data.get("data", [])
        ]
        return json.dumps(out, ensure_ascii=False, indent=2)
    except Exception as e:  # noqa: BLE001
        return _err(e)


@mcp.tool()
def estimate_audience(
    interest_ids: list[str] | None = None,
    geo_countries: list[str] | None = None,
    age_min: int = 18,
    age_max: int = 65,
) -> str:
    """Stima il pubblico raggiungibile per un targeting (interessi + geo + età).

    Args:
        interest_ids: id interesse (da search_interests).
        geo_countries: codici paese (default ["IT"]).
        age_min/age_max: fascia d'età.
    Ritorna la delivery estimate di Meta (incl. stima utenti mensili attivi).
    """
    try:
        targeting = {
            "geo_locations": {"countries": geo_countries or ["IT"]},
            "age_min": age_min,
            "age_max": age_max,
        }
        if interest_ids:
            targeting["flexible_spec"] = [{"interests": [{"id": i} for i in interest_ids]}]
        data = _get(f"{_acct()}/delivery_estimate",
                    {"optimization_goal": "REACH", "targeting_spec": json.dumps(targeting)})
        return json.dumps(data.get("data", data), ensure_ascii=False, indent=2)
    except Exception as e:  # noqa: BLE001
        return _err(e)


@mcp.tool()
def create_adset(
    campaign_id: str,
    name: str,
    daily_budget_eur: float,
    interest_ids: list[str],
    geo_countries: list[str] | None = None,
    age_min: int = 18,
    age_max: int = 65,
    optimization_goal: str = "REACH",
    billing_event: str = "IMPRESSIONS",
) -> str:
    """[SCRITTURA — richiede approvazione esplicita, REGOLA #10] Crea un GRUPPO DI
    INSERZIONI (ad set) con targeting per interessi. Creato in stato **PAUSED**.

    NON chiamare senza aver mostrato all'utente una bozza (campagna, budget, targeting)
    e averne avuto conferma esplicita. daily_budget in EURO (convertito in centesimi).
    """
    try:
        targeting = {
            "geo_locations": {"countries": geo_countries or ["IT"]},
            "age_min": age_min,
            "age_max": age_max,
            "flexible_spec": [{"interests": [{"id": i} for i in interest_ids]}],
        }
        payload = {
            "name": name,
            "campaign_id": campaign_id,
            "daily_budget": int(round(daily_budget_eur * 100)),
            "billing_event": billing_event,
            "optimization_goal": optimization_goal,
            "targeting": json.dumps(targeting),
            "status": "PAUSED",
            "access_token": _token(),
        }
        r = requests.post(f"{GRAPH}/{_acct()}/adsets", data=payload, timeout=40)
        data = r.json()
        if "error" in data:
            raise RuntimeError(f"Errore Meta: {data['error'].get('message', data['error'])}")
        return json.dumps(data, ensure_ascii=False)
    except Exception as e:  # noqa: BLE001
        return _err(e)


def main():
    mcp.run(transport="stdio")


if __name__ == "__main__":
    main()
