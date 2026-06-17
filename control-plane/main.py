"""
Fluxgate Enterprise API Gateway
Module: Control Plane Entrypoint & MCP Server
"""

import asyncio
import logging
from mcp.server.fastmcp import FastMCP
from src.guard import SecurityGuard

# Configure structured logging
logging.basicConfig(
    level=logging.INFO, format="%(asctime)s - %(levelname)s - %(message)s"
)

# Initialize the stateful Security Guard and the MCP Server
guard = SecurityGuard()
mcp = FastMCP("Fluxgate_SOC_Agent")

# ==========================================
# MCP TOOLS (For the LLM Sysadmin)
# ==========================================


@mcp.tool()
async def get_gateway_metrics() -> str:
    """Fetches the live token bucket capacities for all active identities."""
    metrics = await guard.fetch_gateway_metrics()
    return f"Live Gateway State: {metrics}"


@mcp.tool()
async def get_security_incidents() -> str:
    """Retrieves the log of algorithmic rate-limiting interventions and AI reports."""
    if not guard.incident_logs:
        return "No security anomalies detected recently."
    return "\n".join(guard.incident_logs)


@mcp.tool()
async def pardon_identity(identifier: str) -> str:
    """Manually restores a throttled identifier to full capacity (5000 tokens)."""
    return await guard.pardon_identity(identifier)


# ==========================================
# ORCHESTRATION
# ==========================================


async def main():
    # 1. Start the algorithmic background monitor
    asyncio.create_task(guard.run_algorithmic_loop())

    # 2. Start the MCP Server over SSE (port 8000)
    logging.info("MCP Sysadmin Server booting on SSE port 8000...")
    await mcp.run_sse_async(host="0.0.0.0", port=8000)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        logging.info("Control Plane gracefully shutting down.")
