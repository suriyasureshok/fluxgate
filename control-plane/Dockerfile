# Use the official, minimal Python 3.11 slim image (matches Debian Bookworm)
FROM python:3.11-slim-bookworm

# --- 1. Python Environment Standards ---
# Prevent Python from writing .pyc files to disk (reduces container size)
ENV PYTHONDONTWRITEBYTECODE=1
# Force Python stdout and stderr streams to be unbuffered so logs appear instantly
ENV PYTHONUNBUFFERED=1

# --- 2. System Dependencies & Security ---
WORKDIR /app

# Create a non-root system user to run the application securely
RUN useradd -m -s /bin/bash fluxgate_agent

# --- 3. Application Dependencies ---
COPY requirements.txt .

# Upgrade pip and install dependencies without caching them to keep the image lean
RUN pip install --no-cache-dir --upgrade pip && \
    pip install --no-cache-dir -r requirements.txt

# --- 4. Application Source Code ---
# Copy the application code and explicitly grant ownership to the non-root user
COPY --chown=fluxgate_agent:fluxgate_agent . .

# Secure the container by switching execution to the non-root user
USER fluxgate_agent

# The MCP Server operates over SSE on port 8000
EXPOSE 8000

# Execute the cleanly encapsulated orchestrator
CMD ["python", "main.py"]