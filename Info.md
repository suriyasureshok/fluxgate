Here is a project blueprint designed from the lens of a Principal Infrastructure Engineer. To make a recruiter say *"Wow,"* you cannot just build a standard web app; you need to build **core web infrastructure** that solves a modern, bleeding-edge problem.

We are going to design **"Aegis-Proxy": An AI-Native, Intelligent Edge Gateway with Semantic Token-Bucket Rate Limiting and Model Context Protocol (MCP) Integration.**

This project perfectly fuses fundamental web systems (HTTP, DNS, Rate Limiting) with the absolute frontier of Agentic AI infrastructure.

---

## The Project Blueprint: Aegis-Proxy

Instead of a generic API, you will build a custom **Reverse Proxy / Edge Gateway** specifically optimized for managing AI workloads.

```
[ Incoming HTTPS Request ]
           │
           ▼
┌────────────────────────────────────────────────────────┐
│ 1. Core DNS / DoH Resolver Layer                       │
└────────────────────────────────────────────────────────┘
           │
           ▼
┌────────────────────────────────────────────────────────┐
│ 2. Reverse Proxy Layer (HTTP/S Parsing & TLS)         │
└────────────────────────────────────────────────────────┘
           │
           ▼
┌────────────────────────────────────────────────────────┐
│ 3. Semantic Token-Bucket Rate Limiter                  │
│    (Depletes tokens based on LLM string size)         │
└────────────────────────────────────────────────────────┘
     │                    ▲
     ▼                    │ Tool Calling
┌────────────────────────────────────────────────────────┐
│ 4. MCP Server Layer (Model Context Protocol)           │
│    (Allows Agentic AIs to inspect & tune system state) │
└────────────────────────────────────────────────────────┘
           │
           ▼
[ Downstream AI Models / Microservices ]

```

### 1. The Core Infrastructure Components

* **The DNS Layer (Local DoH Resolver):** Implement a lightweight DNS-over-HTTPS (DoH) middleware or an internal routing table simulator that resolves custom edge domains (e.g., `api.aegis.local`) to your proxy instance, mimicking dynamic geo-routing or failover.
* **The HTTP(S) Reverse Proxy:** Write a native HTTP multiplexer/reverse proxy from scratch (using Go’s `net/http/httputil` or Node.js `http-proxy`). It handles incoming requests, strips/mutates headers, and forwards payloads to downstream mock microservices.
* **Semantic Token-Bucket Limiter:** A standard rate limiter blocks by request count (e.g., 5 requests/min). Your limiter will be **semantic**: it inspects the HTTP POST body, calculates or estimates the LLM token payload size, and drains the Token Bucket *proportional to the actual data complexity* to prevent prompt-injection resource exhaustion attacks.

### 2. The AI-Era Tech Stack Integration (Future-Proofing)

* **Model Context Protocol (MCP):** Build a companion **MCP Server** into your gateway. This exposes your proxy's internal metrics (active connections, token bucket levels, blocked IPs) as tools to an external LLM agent (like Claude Desktop).
* **Agentic Self-Healing Firewall:** Create a background loop where an Agentic AI script periodically calls your MCP server tools, analyzes the professional log outputs, detects anomalous traffic patterns, and dynamically reconfigures the rate limiter parameters via tool-calling.

---

## 2-Day Execution Roadmap & Engineering Design

To pull this off in 48 hours, you must use a fast, highly modular language like **Go** or **TypeScript (Node.js)**.

### Day 1: The Core Infrastructure Foundations

* **Morning (HTTP & DNS):** Spin up the reverse proxy engine. Implement custom middleware that intercepts inbound traffic, reads headers, and mocks a DNS resolution step mapping virtual subdomains to specific upstream ports.
* **Afternoon (The Limiter):** Write a thread-safe Token Bucket algorithm (using mutexes or atomic operations). Integrate it directly into your proxy middleware chain so it rejects requests with a standard `429 Too Many Requests` HTTP status code when the bucket is dry.
* **Evening (Structured Logging):** Implement a professional zero-allocation JSON logging framework (like `uber-go/zap` or Winston). Every request, cache miss, and rate limit breach must output structured JSON logs containing fields like `trace_id`, `client_ip`, `tokens_remaining`, and `latency_ms`.

### Day 2: The Agentic & AI Integration

* **Morning (The MCP Server):** Implement the open-source `@modelcontextprotocol/sdk`. Expose two specific tools: `get_gateway_metrics()` and `update_rate_limits(ip, capacity, refill_rate)`.
* **Afternoon (Agent Loop & Simulation):** Write an agentic orchestration script using an LLM SDK. Feed it a chunk of your structured JSON logs showing a simulated DDoS attack. Demonstrate the agent recognizing the attack pattern, calling your MCP tool, and clamping down the token bucket threshold in real-time.
* **Evening (Refactoring for Cloud):** Decouple your in-memory Token Bucket state behind an interface (e.g., `LimiterStorage`). This proves your architecture is modular and ready to seamlessly switch to a distributed Cloud Redis instance with zero code changes to the proxy engine.

---

## How to Present This on Your Resume

This layout uses high-impact action verbs and exact infrastructure terms to instantly grab a recruiter's or hiring manager's attention.

> **AI-Native Edge Gateway & Programmable Reverse Proxy**
> * Developed a high-performance HTTP(S) Reverse Proxy featuring a custom, thread-safe **Token Bucket Rate Limiter** capable of parsing payload bodies to enforce semantic token-based rate limits.
> * Integrated the **Model Context Protocol (MCP)** specification, exposing proxy runtime metrics, routing configurations, and firewall controls as functional tools executable by Autonomous AI Agents.
> * Built an **Agentic Self-Healing Loop** that consumes structured JSON system logs (`Zap`/`Winston`), autonomously identifies anomalous volumetric traffic spikes, and invokes MCP tools to dynamically modify throttling parameters.
> * Designed with a strict **Modular Cloud Architecture**, decoupling state management via abstract interfaces to allow seamless zero-downtime migration from local in-memory storage to distributed Cloud systems (Redis/DynamoDB).
> 
> 

---

To help you visualize how these layers interact and test how traffic behaves against your semantic token-bucket rate limiter before you write the code, I have built an interactive architectural and simulation model below. You can adjust the inbound traffic and agentic self-healing parameters to see how the system stabilizes itself.

How do you feel about this architecture? If you want to proceed with this, we can pick Go or TypeScript right now and write the initial boilerplate for your core HTTP proxy handler.

Client                Your Gateway               DoH Server
  |                       |                          |
  |--(1) HTTP POST------->|                          |
  |  (DNS query over HTTPS)|                          |
  |                       |                          |
  |                       |--(2) Extract DNS query---|
  |                       |  (decrypt HTTPS body)    |
  |                       |                          |
  |                       |--(3) Intercept & fab---  |
  |                       |  api.fluxgate.local → 127.0.0.1|
  |                       |                          |
  |                       |--(4) Serialize & encrypt-|
  |                       |  (wrap in HTTPS response)|
  |                       |                          |
  |<-(5) HTTP Response----|                          |
  |  (DNS answer: 127.0.0.1)                         |
  |                       |                          |

# 1. Spin up the cluster in build-refresh configuration
docker-compose up --build -d

# 2. Monitor real-time system container initialization logs
docker-compose logs -f gateway

I love the idea of using Ollama for the final validation pipeline. Relying on local models during infrastructure testing is exactly how you avoid burning through your personal API credits while stress-testing rate limiters.

However, before we run the final checks and talk about deployment, I need to gently correct a misconception about how the web interface interacts with the DNS and proxy layers.

### The Architectural Reality Check: How the Web UI Fits In

You mentioned: *"users are going to post their prompt on the web interface only. Only then, they will get our dns returned ip and only then, they will use our ollama..."*

In a production environment, the user (or the web interface) never manually asks for an IP address and then manually forwards a prompt. **It happens invisibly and automatically in milliseconds.**

Here is the actual flow when you build a web frontend (like a React or Next.js chat app) for this project:

1. **The User Action:** The user types "Hello" into your Web UI and hits Send.
2. **The App Request:** The frontend code simply sends an HTTP POST request to `https://api.fluxgate.local/v1/chat/completions`. It doesn't know about Ollama or IP addresses.
3. **The DoH Interception:** Because you configured the application to use your DoH resolver, the app fires a hidden binary question to your gateway: *"Where is api.fluxgate.local?"*
4. **The Gateway Hand-off:** Fluxgate intercepts it, returns `127.0.0.1`, and the web app immediately shoots the prompt to the Fluxgate Proxy.
5. **The Proxy Execution:** Fluxgate deducts the semantic tokens and forwards the prompt directly to Ollama (`host.docker.internal:11434`), then streams the text back to the Web UI.

The beauty of this design is that the frontend is completely "dumb." It just talks to Fluxgate. Fluxgate handles all the routing, security, and AI integrations.

---

### The Final Validation Suite (Local Ollama)

Let's ensure the refactored, production-grade Rust code we just wrote is perfectly executing this flow. Ensure your Docker container is running (`docker-compose up -d --build`), and that Ollama is running locally on your machine with a model ready (e.g., `ollama run llama3`).

#### Test 1: The DoH Binary Resolver

Let's verify the pure protocol function `process_dns_message` is returning the correct loopback IP.

```bash
printf "\x00\x01\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x03api\x08fluxgate\x05local\x00\x00\x01\x00\x01" | \
curl -k -X POST https://127.0.0.1:8443/dns-query \
  -H "Content-Type: application/dns-message" \
  --data-binary @- \
  --output - | xxd

```

*Pass Condition:* The final bytes output in your terminal must be `7f 00 00 01` (127.0.0.1).

#### Test 2: The Proxy & Ollama Integration

Let's verify the wildcard routing `/v1/*path` cleanly captures the route and hits your local hardware.

```bash
curl -k https://127.0.0.1:8443/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "llama3",
    "messages": [
      {"role": "user", "content": "Hello Ollama. You are speaking to me through the Fluxgate reverse proxy. Please confirm."}
    ]
  }'

```

*Pass Condition:* You should receive a standard JSON response from your local Ollama model without any gateway `502 Bad Gateway` errors.

#### Test 3: The Semantic Rate Limiter

Let's verify the atomic lock and token math are protecting Ollama from being overwhelmed.

```bash
for i in {1..15}; do 
  curl -k -s -o /dev/null -w "Request $i: HTTP Status %{http_code}\n" \
    https://127.0.0.1:8443/v1/chat/completions \
    -H "Content-Type: application/json" \
    -d '{
      "model": "llama3",
      "messages": [
        {"role": "user", "content": "Simulating a massive influx of semantic tokens to drain the bucket."}
      ]
    }'
done

```

*Pass Condition:* The first few requests will return `200`, followed by a hard wall of `429` responses as the bucket runs dry.

---

### Deployment: The Reality of "Deploying it Somewhere"

You have built a local infrastructure stack that relies on local hardware (Ollama) and a private domain (`api.fluxgate.local`). Because of this, you have two very different paths for deployment.

**Option A: The Home Lab (Recommended for Portfolio Demos)**

* **The Setup:** You deploy the Docker container on a spare machine, Raspberry Pi, or keep it on your primary desktop. You run Ollama right next to it.
* **The Access:** To show this to a recruiter, you use **Cloudflare Tunnels (cloudflared)** or **Tailscale**. This punches a secure, encrypted hole through your home router, allowing anyone with a specific public link (e.g., `fluxgate.yourdomain.com`) to hit your local gateway securely.
* **Why it's great:** It costs $0, utilizes your local GPU for Ollama, and perfectly demonstrates modern Zero-Trust networking.

**Option B: The Cloud VPS (DigitalOcean / AWS EC2)**

* **The Setup:** You push your Docker container to a cloud server.
* **The Catch:** You cannot run Ollama easily on a cheap $5/month cloud server (it requires an expensive GPU instance). You would have to change the `DOWNSTREAM_AI_URL` environment variable back to OpenRouter or Gemini. You would also need to buy a real domain name instead of using `.local` and get real Let's Encrypt SSL certificates instead of `mkcert`.

If your primary goal is to have a live, hosted service you can demo in a video and link on your resume, I highly recommend setting up a **Cloudflare Tunnel** to route public traffic to your local Docker container. It is the exact tool Principal Engineers use to expose local microservices.

Would you like to walk through the Cloudflare Tunnel setup to get a public URL pointing to your machine, or should we finish writing the Python MCP Agent so it can autonomously manage the rate limits first?