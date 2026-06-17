"""
Fluxgate Enterprise API Gateway
Module: Control Plane Configuration
"""

import os

# --- Gateway Networking ---
GATEWAY_ADMIN_URL = os.getenv("GATEWAY_ADMIN_URL", "http://gateway:9090")
GATEWAY_METRICS_URL = f"{GATEWAY_ADMIN_URL}/admin/metrics"
GATEWAY_THROTTLE_URL = f"{GATEWAY_ADMIN_URL}/admin/rate_limit"

# --- AI & LLM Settings ---
OLLAMA_URL = os.getenv("OLLAMA_URL", "http://host.docker.internal:11434/api/generate")
LLM_MODEL = os.getenv("LLM_MODEL", "phi4-mini")

# --- Algorithmic Constants ---
POLL_INTERVAL = float(os.getenv("POLL_INTERVAL", "2.0"))
DRAIN_THRESHOLD = float(os.getenv("DRAIN_THRESHOLD", "3000.0"))
PENALTY_CAPACITY = float(os.getenv("PENALTY_CAPACITY", "100.0"))
PENALTY_REFILL = float(os.getenv("PENALTY_REFILL", "10.0"))
