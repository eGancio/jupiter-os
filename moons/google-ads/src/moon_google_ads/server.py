# SPDX-License-Identifier: AGPL-3.0-or-later
# Copyright (C) 2026 Edoardo Mancinelli

"""Moon Google Ads — MCP server (stdio) for JupiterOS.

Exposes the Google Ads capabilities the user needs, under the single "google-ads"
moon, using the credentials JupiterOS materializes from the OS keyring into the
YAML file pointed to by ``GOOGLE_ADS_CREDENTIALS`` (see crate::ads_credentials).

Why this exists: the official ``google_ads_mcp`` server has NO Keyword Planner
tool. This server adds ``generate_keyword_ideas`` (the #1 need) alongside GAQL
reporting, so everything lives in one moon.

Design notes:
- **stdio transport** so it reuses the sidecar's env injection (no Rust changes).
- **Lazy client init**: the GoogleAdsClient is built on the first tool call, not at
  startup, so the MCP handshake answers instantly and never times out.
"""

import json
import os
from functools import lru_cache

from dotenv import load_dotenv
from mcp.server.fastmcp import FastMCP

load_dotenv()

mcp = FastMCP("google-ads")

# Italy geo target constant + Italian language constant (sensible defaults).
_DEFAULT_GEO = "2380"      # Italy
_DEFAULT_LANG = "1004"     # Italian


@lru_cache(maxsize=1)
def _client():
    """Build (once) and cache the GoogleAdsClient from the YAML whose path is in
    GOOGLE_ADS_CREDENTIALS. Imported lazily so the server starts even without the
    google-ads package present at import time, and so the MCP handshake is instant."""
    from google.ads.googleads.client import GoogleAdsClient

    path = os.environ.get("GOOGLE_ADS_CREDENTIALS")
    if not path or not os.path.exists(path):
        raise RuntimeError(
            "Credenziali Google Ads non trovate. Inseriscile in JupiterOS "
            "(Service Detail → Google Ads) e riavvia."
        )
    return GoogleAdsClient.load_from_storage(path)


def _default_customer_id() -> str:
    cid = getattr(_client(), "login_customer_id", None)
    return str(cid).replace("-", "") if cid else ""


def _err(e: Exception) -> str:
    """Readable error string (unwraps GoogleAdsException into its messages)."""
    try:
        from google.ads.googleads.errors import GoogleAdsException

        if isinstance(e, GoogleAdsException):
            msgs = [err.message for err in e.failure.errors]
            return "Errore Google Ads: " + " | ".join(msgs)
    except Exception:
        pass
    return f"Errore: {type(e).__name__}: {e}"


@mcp.tool()
def generate_keyword_ideas(
    seed_keywords: list[str] | None = None,
    page_url: str = "",
    geo_target_ids: list[str] | None = None,
    language_id: str = _DEFAULT_LANG,
    limit: int = 50,
) -> str:
    """Genera IDEE di parole chiave con VOLUMI di ricerca medi mensili e
    competizione, dal Keyword Planner di Google Ads.

    Args:
        seed_keywords: parole-seme da cui espandere (es. ["fotovoltaico","accumulo"]).
        page_url: in alternativa/aggiunta, una URL da cui estrarre temi.
        geo_target_ids: ID geo target (default Italia=2380; es. regioni IT).
        language_id: ID lingua (default italiano=1004).
        limit: numero massimo di idee restituite (ordinate per volume).
    Ritorna JSON: keyword, avg_monthly_searches, competition, bid min/max (EUR).
    """
    try:
        client = _client()
        svc = client.get_service("KeywordPlanIdeaService")
        req = client.get_type("GenerateKeywordIdeasRequest")
        req.customer_id = _default_customer_id()
        req.language = f"languageConstants/{language_id}"
        for g in (geo_target_ids or [_DEFAULT_GEO]):
            req.geo_target_constants.append(f"geoTargetConstants/{g}")
        req.keyword_plan_network = (
            client.enums.KeywordPlanNetworkEnum.GOOGLE_SEARCH
        )
        seeds = [s for s in (seed_keywords or []) if s.strip()]
        if seeds and page_url:
            req.keyword_and_url_seed.url = page_url
            req.keyword_and_url_seed.keywords.extend(seeds)
        elif page_url:
            req.url_seed.url = page_url
        elif seeds:
            req.keyword_seed.keywords.extend(seeds)
        else:
            return "Errore: fornisci almeno seed_keywords o page_url."

        rows = []
        for idea in svc.generate_keyword_ideas(request=req):
            m = idea.keyword_idea_metrics
            rows.append(
                {
                    "keyword": idea.text,
                    "avg_monthly_searches": m.avg_monthly_searches or 0,
                    "competition": m.competition.name,
                    "bid_low_eur": round((m.low_top_of_page_bid_micros or 0) / 1_000_000, 2),
                    "bid_high_eur": round((m.high_top_of_page_bid_micros or 0) / 1_000_000, 2),
                }
            )
        rows.sort(key=lambda r: r["avg_monthly_searches"], reverse=True)
        return json.dumps(rows[:limit], ensure_ascii=False, indent=2)
    except Exception as e:  # noqa: BLE001
        return _err(e)


@mcp.tool()
def execute_gaql(query: str, customer_id: str = "") -> str:
    """Esegue una query GAQL (Google Ads Query Language) e ritorna le righe —
    per KPI/report di campagne, ad group, keyword (dati che già esistono).

    Args:
        query: query GAQL (es. "SELECT campaign.name, metrics.clicks,
            metrics.cost_micros FROM campaign WHERE segments.date DURING LAST_30_DAYS").
        customer_id: account (senza trattini); default = account di login.
    """
    try:
        from google.protobuf.json_format import MessageToDict

        client = _client()
        cid = (customer_id or _default_customer_id()).replace("-", "")
        if not cid:
            return "Errore: customer_id mancante e nessun login_customer_id nelle credenziali."
        ga = client.get_service("GoogleAdsService")
        out = []
        for batch in ga.search_stream(customer_id=cid, query=query):
            for row in batch.results:
                out.append(MessageToDict(row._pb))
        return json.dumps(out, ensure_ascii=False, default=str)
    except Exception as e:  # noqa: BLE001
        return _err(e)


@mcp.tool()
def list_accessible_accounts() -> str:
    """Elenca gli account Google Ads accessibili con le credenziali correnti
    (resource name = customers/<id>)."""
    try:
        client = _client()
        svc = client.get_service("CustomerService")
        res = svc.list_accessible_customers()
        return json.dumps(list(res.resource_names), ensure_ascii=False)
    except Exception as e:  # noqa: BLE001
        return _err(e)


def main():
    """Entry point — stdio transport so the chat sidecar connects directly."""
    mcp.run(transport="stdio")


if __name__ == "__main__":
    main()
