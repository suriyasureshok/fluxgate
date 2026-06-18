"""
Fluxgate Enterprise API Gateway
Module: Algorithmic Security Guard & Incident Reporter
"""

import asyncio
import logging
from typing import Dict, Optional
import httpx
from src import config
from src.storage import IncidentStore

logger = logging.getLogger("fluxgate.security_guard")


class SecurityGuard:
    """
    Orchestrates the 'Hot Path' (real-time throttling) and
    'Cold Path' (LLM analysis) for API security.
    """

    def __init__(self, store: IncidentStore) -> None:
        self.store = store
        self._client: Optional[httpx.AsyncClient] = None
        self.previous_state: Dict[str, float] = {}

    async def initialize(self) -> None:
        """Sets up the persistent connection pool."""
        self._client = httpx.AsyncClient(timeout=10.0)
        logger.info("SecurityGuard connection pool initialized.")

    async def close(self) -> None:
        """Tears down connections gracefully."""
        if self._client:
            await self._client.aclose()
            logger.info("SecurityGuard connection pool closed.")

    @property
    def client(self) -> httpx.AsyncClient:
        """Ensures the client is initialized before access."""
        if self._client is None:
            raise RuntimeError("SecurityGuard accessed before initialization.")
        return self._client

    async def _execute_throttle(self, identifier: str) -> None:
        """Sends throttle signal to Data Plane with robust error handling."""
        try:
            response = await self.client.post(
                config.GATEWAY_THROTTLE_URL,
                json={
                    "identifier": identifier,
                    "capacity": config.PENALTY_CAPACITY,
                    "refill_rate": config.PENALTY_REFILL,
                },
            )
            response.raise_for_status()
            logger.warning(f"Throttle applied successfully to {identifier}")
        except httpx.HTTPError as e:
            logger.error(f"Failed to throttle {identifier}: {e}")

    async def generate_report_worker(self, identifier: str, burned: float) -> None:
        """The Cold Path: Offloaded worker for LLM inference (non-blocking)."""
        prompt = (
            f"Security Analyst Report:\n"
            f"Identity: {identifier}\n"
            f"Issue: Context Flood. Burned {burned} tokens.\n"
            f"Action: Throttled to {config.PENALTY_CAPACITY}."
        )
        try:
            response = await self.client.post(
                config.OLLAMA_URL,
                json={"model": config.LLM_MODEL, "prompt": prompt, "stream": False},
                timeout=30.0,
            )
            response.raise_for_status()
            report = response.json().get("response", "No summary generated.")

            # Persist to central storage
            await self.store.add_incident(identifier, report)
        except Exception as e:
            logger.error(f"Cold path analysis worker failed for {identifier}: {e}")

    async def fetch_gateway_metrics(self) -> Dict[str, float]:
        """Fetches telemetry from the Rust Data Plane."""
        try:
            response = await self.client.get(config.GATEWAY_METRICS_URL)
            response.raise_for_status()
            return response.json()
        except Exception as e:
            logger.error(f"Telemetry fetch failed: {e}")
            return {}

    async def run_algorithmic_loop(self) -> None:
        """
        The Hot Path: Continuous monitoring loop.
        Processes state changes every POLL_INTERVAL seconds.
        """
        logger.info("Monitoring loop active.")
        while True:
            try:
                current_state = await self.fetch_gateway_metrics()

                # Guard against empty state or connection errors
                if not current_state:
                    await asyncio.sleep(config.POLL_INTERVAL)
                    continue

                for identifier, capacity in current_state.items():
                    # Only calculate diffs for known identifiers
                    if identifier in self.previous_state:
                        burned = self.previous_state[identifier] - capacity

                        # Trigger guard if threshold breached
                        if burned > config.DRAIN_THRESHOLD:
                            logger.warning(
                                f"Security Alert: {identifier} burned {burned} tokens."
                            )
                            await self._execute_throttle(identifier)

                            # Offload inference to worker queue
                            asyncio.create_task(
                                self.generate_report_worker(identifier, burned)
                            )

                self.previous_state = current_state

            except Exception as e:
                # Log critical loop failure but keep the service running
                logger.error(f"Algorithmic loop execution error: {e}", exc_info=True)

            await asyncio.sleep(config.POLL_INTERVAL)
