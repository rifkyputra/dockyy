# TODO — Coolify Feature Parity

Dockyy's goal: **single binary, ~10MB RAM, zero external dependencies** — a lightweight Coolify alternative.
This list tracks gaps between Dockyy and [Coolify](https://github.com/coollabsio/coolify).

Items are grouped by theme and roughly ordered by impact. Features that conflict with the lightweight
philosophy are marked `[HEAVY]` (needs significant infrastructure) or `[OUT-OF-SCOPE]` (not a fit for this project).

---

## 🚀 Deployment & Build

- [ ] **Build strategies** — support multiple build methods per repo:
  - [ ] Nixpacks (auto-detect language/framework, no Dockerfile needed)
  - [ ] Custom Dockerfile (currently assumed but not explicitly supported)
  - [ ] Docker Compose file deployment (`docker compose up`)
  - [ ] Pre-built Docker image deploy (pull from registry, skip build)
  - [ ] Static site build + serve (e.g. via embedded Nginx/Caddy)
- [ ] **Environment variables** — inject env vars into deployments (UI + API CRUD, per-repo)
- [ ] **Rollback** — one-click revert to a previous successful deployment
- [ ] **Pull Request preview environments** — auto-deploy on PR open/update, teardown on PR close
- [ ] **Branch-based deployments** — deploy different branches independently, not just `default_branch`
- [ ] **Deployment cancellation** — cancel an in-progress build/deploy job
- [ ] **Deployment retention policy** — configurable number of past deployments to keep
- [ ] **Resource limits per deployment** — CPU, memory, restart policy (via Docker API)
- [ ] **Health checks** — configure HTTP or command health checks per deployment; block traffic until healthy

---

## 🌐 Networking & Reverse Proxy

- [ ] **Automatic reverse proxy** — Traefik or Caddy sidecar; route traffic to containers by domain
- [ ] **Custom domain assignment** — map a domain/subdomain to a deployment via UI
- [ ] **Automatic TLS (Let's Encrypt)** — provision and renew SSL certs for assigned domains
- [ ] **HTTP→HTTPS redirect** — enforce HTTPS for all routed services
- [ ] **Wildcard domain support** — `<uuid>.yourdomain.com` per-server wildcard routing
- [ ] **Port exposure management** — choose which container ports to expose/publish
- [ ] **Docker network management** — create/delete networks, connect containers to networks

---

## 🖥️ Server & Infrastructure Management

- [ ] **Remote server management via SSH** — add, validate, and manage multiple servers (not just localhost Docker)
- [ ] **SSH key management** — create, store, and associate SSH keypairs with servers
- [ ] **Server health monitoring** — periodic checks for connectivity, disk, CPU, RAM thresholds
- [ ] **Automated Docker cleanup** — scheduled removal of dangling images/volumes/containers
- [ ] **Build server support** — delegate builds to a dedicated build node
- [ ] **Cloud provider integration** `[HEAVY]` — Hetzner/AWS/DigitalOcean server provisioning via API

---

## 🗄️ Database & Managed Services

- [ ] **One-click database provisioning** — spin up standalone containers for:
  - [ ] PostgreSQL
  - [ ] MySQL / MariaDB
  - [ ] MongoDB
  - [ ] Redis
- [ ] **Database environment variable injection** — auto-populate `DATABASE_URL` etc. into linked apps
- [ ] **Database backup & restore** — scheduled dumps, S3-compatible storage target, one-click restore
- [ ] **One-click services catalog** — curated templates for self-hosted apps (n8n, Gitea, Uptime Kuma, etc.)

---

## 🔌 Git Provider Integrations

- [ ] **GitLab webhooks** — deploy on push from GitLab repos
- [ ] **Bitbucket webhooks** — deploy on push from Bitbucket repos
- [ ] **Gitea / self-hosted Git** — webhook support for Gitea and generic Git servers
- [ ] **GitHub App integration** — proper OAuth GitHub App (vs. raw webhook secret) for private repos
- [ ] **Deploy key management** — generate and register SSH deploy keys per repo

---

## 📊 Monitoring & Observability

- [ ] **Real-time log streaming** — WebSocket or SSE endpoint for live container logs (currently polling)
- [ ] **Server/container metrics** — CPU, RAM, disk usage charts (via Docker stats API; no external agents needed)
- [ ] **Deployment log history** — persist full build logs per deployment (currently only latest)
- [ ] **Container inspect / stats** — expose `docker inspect` and `docker stats` data in the UI
- [ ] **Health check status in dashboard** — show per-container health check state

---

## 🔔 Notifications & Alerting

- [ ] **Email notifications** (SMTP) — deployment success/failure, server alerts
- [ ] **Slack / Discord webhook notifications** — post deployment events to a channel
- [ ] **Telegram bot notifications**
- [ ] **Generic HTTP webhook notifications** — POST deployment events to a custom URL
- [ ] **Configurable alert thresholds** — disk/CPU/RAM alerts per server

---

## 👥 User Management

- [ ] **Multi-user support** — add additional admin/read-only accounts (currently single `admin` user)
- [ ] **Role-based access control** — viewer, deployer, admin roles
- [ ] **Team / organization support** `[HEAVY]` — multiple isolated teams with their own projects
- [ ] **Personal API tokens** — per-user scoped tokens (read, write, deploy)
- [ ] **OAuth login** — GitHub / GitLab / Google SSO

---

## 🔧 Container Management Improvements

- [ ] **Container terminal** — browser-based `docker exec` shell into a running container
- [ ] **Docker image management** — list images, pull from registry, delete unused images
- [ ] **Volume management** — list, create, delete, inspect persistent volumes
- [ ] **Docker Compose stacks** — manage multi-container stacks as a single unit
- [ ] **Container environment variable editing** — view/edit env vars of a running container
- [ ] **Container rename / relabel** — set labels and names from the UI

---

## 🔁 Scheduled Tasks

- [ ] **Cron job support** — define cron-syntax tasks that run inside a deployment's container
- [ ] **Scheduled task history** — log execution results per task run
- [ ] **Scheduled database backups** — cron-based dump with configurable retention

---

## 🔍 UX & API

- [ ] **Global search** — search across repos, deployments, containers in one box
- [ ] **Resource tagging** — tag repos/deployments for organization and filtering
- [ ] **Versioned REST API** — `/api/v1/` prefix with stable versioning contract
- [ ] **OpenAPI / Swagger spec** — auto-generated API documentation
- [ ] **Paginated list endpoints** — add `limit`/`offset` or cursor pagination to all list routes
- [ ] **Audit log** — record who did what (deploy, stop, delete) with timestamp

---

## 🐋 Docker Orchestration

- [ ] **Docker Swarm support** `[HEAVY]` — deploy services across a multi-node Swarm cluster
- [ ] **Kubernetes support** `[OUT-OF-SCOPE]` — too heavy for the single-binary philosophy

---

## ☁️ Storage & Backup

- [ ] **S3-compatible storage integration** — configure an S3 bucket for backup targets
- [ ] **Volume snapshot / clone** — snapshot named volumes before a deploy for rollback safety

---

## 🔒 Security

- [ ] **Webhook signature validation** — currently GitHub HMAC signature is checked; extend to GitLab/Gitea
- [ ] **Secrets management** — encrypted at-rest storage for env vars / SSH keys in SQLite
- [ ] **Automatic security header middleware** — add HSTS, CSP, X-Frame-Options to all responses
- [ ] **Rate limiting** — protect auth and webhook endpoints from brute-force

---

## 🛠️ Ops & Distribution

- [ ] **Auto-update mechanism** — check for new Dockyy releases and self-upgrade
- [ ] **Structured JSON logging** — machine-readable log output option (`RUST_LOG=json`)
- [ ] **Prometheus metrics endpoint** — expose `/metrics` for external scraping
- [ ] **Configurable job worker concurrency** — tune how many build jobs run in parallel
- [ ] **Graceful shutdown** — drain in-flight jobs before process exit

---

## Notes

- Coolify uses Laravel + PHP + Livewire + Redis + Horizon — a full web framework stack.
  Dockyy's constraint of **single binary + embedded SQLite** means some features (real-time
  broadcast channels, queue workers, Stripe billing, team multi-tenancy) would require
  significant design trade-offs.
- Features marked `[HEAVY]` are technically possible but would inflate binary size or add
  optional runtime dependencies.
- Features marked `[OUT-OF-SCOPE]` are anti-patterns for the lightweight-first design goal.
