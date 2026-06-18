"""
Fluxgate Enterprise API Gateway - Control Plane (MCP)
Module: Main Orchestrator & Entrypoint

This service acts as the Brain of the Fluxgate infrastructure, maintaining
state consistency, persistent security logs, and live telemetry via the
Model Context Protocol (MCP).
"""

import asyncio
import contextlib
import logging
import os
from typing import Any

import uvicorn
from starlette.middleware.cors import CORSMiddleware
from mcp.server.fastmcp import FastMCP

from src.guard import SecurityGuard
from src.storage import IncidentStore

# --- 1. Global Setup & Logging Configuration ---
logging.basicConfig(
    level=logging.INFO, format="%(asctime)s - %(name)s - %(levelname)s - %(message)s"
)
logger = logging.getLogger("fluxgate.control_plane")

# --- 2. Infrastructure Initialization ---
# Fail-fast pattern: Read env vars immediately
REDIS_URL = os.getenv("REDIS_URL", "redis://localhost:6379")

# Dependencies
store = IncidentStore(REDIS_URL)
guard = SecurityGuard(store)
mcp = FastMCP("Fluxgate_SOC_Agent")

# --- 3. MCP Tools (Exposed to AI Agents) ---


@mcp.tool()
async def get_gateway_metrics() -> str:
    """Fetches live token bucket capacities from the Rust Data Plane."""
    metrics = await guard.fetch_gateway_metrics()
    return f"Live Gateway State: {metrics}"


@mcp.tool()
async def get_security_incidents() -> str:
    """Retrieves persistent security audit logs from the Redis store."""
    logs = await store.get_all_incidents()
    if not logs:
        return "No anomalies identified in persistent storage."
    return f"Active Security Incidents: {logs}"


@mcp.tool()
async def pardon_identity(identifier: str) -> str:
    """Administratively lifts a throttle penalty for a specific identifier."""
    return await guard.pardon_identity(identifier)


@mcp.tool()
async def debug_guard_state() -> str:
    """Provides internal structural state for system diagnostics."""
    metrics = await guard.fetch_gateway_metrics()
    return f"Live Redis Metrics: {metrics}\nGuard Previous State Checkpoint: {guard.previous_state}"


# --- 4. Application Lifecycle ---

app = mcp.sse_app()

# Apply robust CORS policy for production deployment
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],  # Restrict this to your specific frontend URL in prod
    allow_methods=["GET", "POST", "OPTIONS"],
    allow_headers=["*"],
)


@contextlib.asynccontextmanager
async def lifespan(starlette_app: Any):
    """
    Manages the lifecycle of the control plane:
    1. Initializes connections (Guard, Redis/Store)
    2. Spawns the background monitoring loop
    3. Handles graceful cleanup on shutdown
    """
    # Initialization
    await guard.initialize()
    logger.info("SecurityGuard initialized.")

    # Background worker: Hot Path Monitoring
    loop_task = asyncio.create_task(guard.run_algorithmic_loop())

    def supervisor_callback(task: asyncio.Task) -> None:
        try:
            task.result()
        except asyncio.CancelledError:
            logger.info("Algorithmic loop task was cancelled.")
        except Exception as exc:
            logger.critical(
                f"CRITICAL: Structural runtime engine collapsed: {exc}", exc_info=True
            )

    loop_task.add_done_callback(supervisor_callback)

    yield  # System is running

    # Graceful Shutdown
    logger.info("Shutdown initiated. Draining background tasks...")
    loop_task.cancel()
    with contextlib.suppress(asyncio.CancelledError):
        await loop_task

    await guard.close()
    logger.info("Shutdown complete.")


app.router.lifespan_context = lifespan

# --- 5. Entrypoint ---


async def main() -> None:
    """Entry point for the Uvicorn server."""
    logger.info("Launching Fluxgate MCP Control Plane on port 8000...")

    config_server = uvicorn.Config(
        app=app, host="0.0.0.0", port=8000, log_level="info", loop="asyncio"
    )
    server = uvicorn.Server(config_server)
    await server.serve()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        logger.info("Process interrupted by user.")
