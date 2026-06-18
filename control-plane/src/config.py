"""
Fluxgate Enterprise API Gateway
Module: Control Plane Configuration

Centralized configuration management. This module handles environment variable
injection and enforces type safety for algorithmic security thresholds.
"""

import os


def _get_float(key: str, default: str) -> float:
    """Helper to safely parse environment variables into floats."""
    try:
        return float(os.getenv(key, default))
    except (ValueError, TypeError):
        return float(default)


# --- Gateway Networking ---
# Base URL for the Rust Data Plane admin interface
GATEWAY_ADMIN_URL = os.getenv("GATEWAY_ADMIN_URL", "http://gateway:9090")
GATEWAY_METRICS_URL = f"{GATEWAY_ADMIN_URL}/admin/metrics"
GATEWAY_THROTTLE_URL = f"{GATEWAY_ADMIN_URL}/admin/rate_limit"

# --- AI & LLM Settings ---
# Endpoint for Ollama generation API
OLLAMA_URL = os.getenv("OLLAMA_URL", "http://host.docker.internal:11434/api/generate")
LLM_MODEL = os.getenv("LLM_MODEL", "phi4-mini")

# --- Algorithmic Constants ---
# POLL_INTERVAL: Frequency of metrics collection in seconds (float)
POLL_INTERVAL = _get_float("POLL_INTERVAL", "2.0")

# DRAIN_THRESHOLD: Max tokens a user can burn between POLL_INTERVALs before triggering an alert (float)
DRAIN_THRESHOLD = _get_float("DRAIN_THRESHOLD", "300.0")

# PENALTY_CAPACITY: The 'Penalty Box' token bucket capacity assigned when throttled (float)
PENALTY_CAPACITY = _get_float("PENALTY_CAPACITY", "100.0")

# PENALTY_REFILL: The rate at which the 'Penalty Box' bucket refills (float)
PENALTY_REFILL = _get_float("PENALTY_REFILL", "10.0")
