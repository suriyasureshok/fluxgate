"""
Fluxgate Enterprise API Gateway
Module: Algorithmic Security Guard & Incident Reporter
"""

import asyncio
import httpx
import logging
import datetime
from typing import Dict, List
from src import config


class SecurityGuard:
    def __init__(self):
        # Encapsulated state (replaces global variables)
        self.previous_state: Dict[str, float] = {}
        self.incident_logs: List[str] = []

    async def fetch_gateway_metrics(self) -> Dict[str, float]:
        """Fetches the live token bucket capacities from the Rust Data Plane."""
        try:
            async with httpx.AsyncClient() as client:
                res = await client.get(config.GATEWAY_METRICS_URL)
                res.raise_for_status()
                return res.json()
        except Exception as e:
            logging.error(f"Failed to fetch metrics from Gateway: {e}")
            return {}

    async def pardon_identity(self, identifier: str) -> str:
        """Manually restores a throttled identifier to full capacity."""
        try:
            async with httpx.AsyncClient() as client:
                res = await client.post(
                    config.GATEWAY_THROTTLE_URL,
                    json={
                        "identifier": identifier,
                        "capacity": 5000.0,
                        "refill_rate": 500.0,
                    },
                )
                res.raise_for_status()
                return f"SUCCESS: {identifier} has been pardoned and restored."
        except Exception as e:
            return f"Failed to pardon {identifier}: {e}"

    async def _execute_throttle(self, client: httpx.AsyncClient, identifier: str):
        """Sends the penalty-box override command to the Rust Gateway."""
        try:
            await client.post(
                config.GATEWAY_THROTTLE_URL,
                json={
                    "identifier": identifier,
                    "capacity": config.PENALTY_CAPACITY,
                    "refill_rate": config.PENALTY_REFILL,
                },
            )
        except Exception as e:
            logging.error(f"Failed to throttle {identifier}: {e}")

    async def _generate_incident_report(self, identifier: str, tokens_burned: float):
        """The Cold Path: Asks the local LLM to generate a human-readable security report."""
        prompt = f"""
        You are an expert Cybersecurity Analyst monitoring an API Gateway.
        An algorithmic guard just blocked a user or API Key identified as '{identifier}'.
        They attempted to consume {tokens_burned} tokens in under {config.POLL_INTERVAL} seconds, triggering a Context Flood alert.
        
        Write a brief, professional 3-sentence incident report. 
        State the threat, the action taken (IP/Key throttled to {config.PENALTY_CAPACITY} capacity), and a recommendation.
        """
        try:
            async with httpx.AsyncClient() as client:
                response = await client.post(
                    config.OLLAMA_URL,
                    json={"model": config.LLM_MODEL, "prompt": prompt, "stream": False},
                    timeout=30.0,
                )

                if response.status_code == 200:
                    report = response.json().get("response", "No response generated.")
                    logging.info(
                        f"\n{'='*50}\nAI SECURITY REPORT\n{'='*50}\n{report}\n{'='*50}"
                    )
                    self.incident_logs.append(f"AI Report for {identifier}: {report}")
                else:
                    logging.error(
                        f"Failed to generate AI report. Status: {response.status_code}"
                    )
        except Exception as e:
            logging.error(f"LLM unreachable for incident reporting: {e}")

    async def run_algorithmic_loop(self):
        """The Hot Path: Continuous monitoring and derivative calculation loop."""
        logging.info("Algorithmic Guard activated. Monitoring token drain velocity...")

        async with httpx.AsyncClient() as client:
            while True:
                current_state = await self.fetch_gateway_metrics()

                if not current_state:
                    await asyncio.sleep(config.POLL_INTERVAL)
                    continue

                for identifier, current_capacity in current_state.items():
                    if identifier in self.previous_state:
                        burned = self.previous_state[identifier] - current_capacity

                        if burned > config.DRAIN_THRESHOLD:
                            logging.warning(
                                f"ANOMALY: {identifier} burned {burned} tokens. Throttling."
                            )

                            # 1. Execute Throttle
                            await self._execute_throttle(client, identifier)

                            # 2. Log for the LLM Sysadmin
                            timestamp = datetime.datetime.now().strftime(
                                "%Y-%m-%d %H:%M:%S"
                            )
                            self.incident_logs.append(
                                f"[{timestamp}] Blocked: {identifier} consumed {burned} tokens in {config.POLL_INTERVAL}s."
                            )

                            # 3. Trigger async AI analysis
                            asyncio.create_task(
                                self._generate_incident_report(identifier, burned)
                            )

                            # 4. Overwrite local state to prevent double-triggering
                            current_state[identifier] = config.PENALTY_CAPACITY

                self.previous_state = current_state
                await asyncio.sleep(config.POLL_INTERVAL)
