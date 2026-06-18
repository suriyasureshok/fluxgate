"""
Fluxgate Enterprise API Gateway
Module: Persistent Incident Storage
"""

import json
import logging
import redis.asyncio as redis
from typing import List, Dict, Any

# Standardized key for incident storage
INCIDENT_LOG_KEY = "fluxgate:incidents"
logger = logging.getLogger("fluxgate.storage")


class IncidentStore:
    """
    Handles persistent storage of security incidents using Redis.
    Implements a capped list (Circular Buffer) to manage memory usage automatically.
    """

    def __init__(self, redis_url: str):
        # We initialize the client but do not connect until first use (lazy)
        # redis.from_url handles internal connection pooling efficiently
        self.client = redis.from_url(redis_url, decode_responses=True)

    async def add_incident(self, identifier: str, report: str) -> bool:
        """
        Persists incident to Redis with error handling.

        Returns:
            bool: True if successful, False if persistence failed (fails-safe).
        """
        try:
            payload = json.dumps({"identifier": identifier, "report": report})

            # Atomic pipeline to push and trim in one RTT
            async with self.client.pipeline(transaction=True) as pipe:
                await pipe.lpush(INCIDENT_LOG_KEY, payload)
                await pipe.ltrim(
                    INCIDENT_LOG_KEY, 0, 99
                )  # Keep only the last 100 entries
                await pipe.execute()

            return True
        except Exception as e:
            # We catch here so the main algorithmic loop doesn't crash if Redis is down
            logger.error(f"Failed to persist incident to Redis: {e}", exc_info=True)
            return False

    async def get_all_incidents(self) -> List[Dict[str, Any]]:
        """
        Retrieves all currently stored incidents.

        Returns:
            List of parsed dictionary objects, empty list if Redis fails.
        """
        try:
            raw_logs = await self.client.lrange(INCIDENT_LOG_KEY, 0, -1)
            return [json.loads(log) for log in raw_logs]
        except Exception as e:
            logger.error(f"Failed to retrieve incidents from Redis: {e}")
            return []
