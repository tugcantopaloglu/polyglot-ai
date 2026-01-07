//! Embedded Web UI Dashboard for Bridge Administration
//!
//! Provides a simple HTTP dashboard for monitoring:
//! - Connected clients
//! - Tool health status
//! - Metrics and statistics
//! - Rate limit status
//! - Quota usage

use std::sync::Arc;
use std::net::SocketAddr;

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, error};

use crate::BridgeState;

/// Dashboard configuration
#[derive(Debug, Clone)]
pub struct DashboardConfig {
    pub enabled: bool,
    pub listen: String,
    pub require_auth: bool,
    pub auth_token: Option<String>,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen: "127.0.0.1:8788".to_string(),
            require_auth: false,
            auth_token: None,
        }
    }
}

/// Start the dashboard HTTP server
pub async fn start_dashboard(config: DashboardConfig, state: Arc<BridgeState>) -> Result<()> {
    if !config.enabled {
        return Ok(());
    }

    let listener = TcpListener::bind(&config.listen).await?;
    info!("Dashboard listening on http://{}", config.listen);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let state = state.clone();
                let config = config.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_http_request(stream, addr, state, &config).await {
                        error!("Dashboard request error: {}", e);
                    }
                });
            }
            Err(e) => error!("Dashboard accept error: {}", e),
        }
    }
}

async fn handle_http_request(
    mut stream: TcpStream,
    _addr: SocketAddr,
    state: Arc<BridgeState>,
    config: &DashboardConfig,
) -> Result<()> {
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse request line
    let first_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();

    if parts.len() < 2 {
        send_response(&mut stream, 400, "text/plain", "Bad Request").await?;
        return Ok(());
    }

    let method = parts[0];
    let path = parts[1];

    // Check auth if required
    if config.require_auth {
        if let Some(expected_token) = &config.auth_token {
            let auth_header = request.lines()
                .find(|l| l.to_lowercase().starts_with("authorization:"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim());

            let provided = auth_header
                .and_then(|h| h.strip_prefix("Bearer "))
                .unwrap_or("");

            if provided != expected_token {
                send_response(&mut stream, 401, "text/plain", "Unauthorized").await?;
                return Ok(());
            }
        }
    }

    // Route requests
    match (method, path) {
        ("GET", "/") | ("GET", "/dashboard") => {
            send_response(&mut stream, 200, "text/html", DASHBOARD_HTML).await?;
        }
        ("GET", "/api/status") => {
            let status = get_status_json(&state);
            send_response(&mut stream, 200, "application/json", &status).await?;
        }
        ("GET", "/api/metrics") => {
            let metrics = get_metrics_json(&state);
            send_response(&mut stream, 200, "application/json", &metrics).await?;
        }
        ("GET", "/api/health") => {
            let health = get_health_json(&state);
            send_response(&mut stream, 200, "application/json", &health).await?;
        }
        ("GET", "/api/quotas") => {
            let quotas = get_quotas_json(&state);
            send_response(&mut stream, 200, "application/json", &quotas).await?;
        }
        ("GET", "/style.css") => {
            send_response(&mut stream, 200, "text/css", DASHBOARD_CSS).await?;
        }
        ("GET", "/script.js") => {
            send_response(&mut stream, 200, "application/javascript", DASHBOARD_JS).await?;
        }
        _ => {
            send_response(&mut stream, 404, "text/plain", "Not Found").await?;
        }
    }

    Ok(())
}

async fn send_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };

    let response = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: {}; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Cache-Control: no-cache\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        status, status_text, content_type, body.len(), body
    );

    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

#[derive(Serialize)]
struct StatusResponse {
    server_time: String,
    uptime_seconds: u64,
    version: &'static str,
    mode: String,
}

fn get_status_json(state: &BridgeState) -> String {
    let cache_stats = state.get_cache_stats();
    let metrics = state.metrics.get_metrics(cache_stats);

    let status = StatusResponse {
        server_time: Utc::now().to_rfc3339(),
        uptime_seconds: metrics.uptime_seconds,
        version: env!("CARGO_PKG_VERSION"),
        mode: format!("{:?}", state.config.mode),
    };

    serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string())
}

#[derive(Serialize)]
struct MetricsResponse {
    active_connections: u32,
    total_requests: u64,
    requests_per_minute: f64,
    cache_hits: u64,
    cache_misses: u64,
    cache_hit_rate: f32,
    cache_entries: u64,
    cache_memory_mb: f64,
    uptime_seconds: u64,
    tool_stats: Vec<ToolStatResponse>,
}

#[derive(Serialize)]
struct ToolStatResponse {
    tool: String,
    total_requests: u64,
    successful: u64,
    failed: u64,
    avg_latency_ms: u32,
    rate_limit_hits: u64,
}

fn get_metrics_json(state: &BridgeState) -> String {
    let cache_stats = state.get_cache_stats();
    let metrics = state.metrics.get_metrics(cache_stats.clone());

    let response = MetricsResponse {
        active_connections: metrics.active_connections,
        total_requests: metrics.total_requests,
        requests_per_minute: metrics.requests_per_minute,
        cache_hits: cache_stats.hits,
        cache_misses: cache_stats.misses,
        cache_hit_rate: cache_stats.hit_rate,
        cache_entries: cache_stats.entries,
        cache_memory_mb: cache_stats.memory_bytes as f64 / 1024.0 / 1024.0,
        uptime_seconds: metrics.uptime_seconds,
        tool_stats: metrics.tool_stats.iter().map(|t| ToolStatResponse {
            tool: t.tool.to_string(),
            total_requests: t.total_requests,
            successful: t.successful_requests,
            failed: t.failed_requests,
            avg_latency_ms: t.avg_latency_ms,
            rate_limit_hits: t.rate_limit_hits,
        }).collect(),
    };

    serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string())
}

#[derive(Serialize)]
struct HealthResponse {
    server_healthy: bool,
    tools: Vec<ToolHealthResponse>,
}

#[derive(Serialize)]
struct ToolHealthResponse {
    tool: String,
    healthy: bool,
    last_check: String,
    latency_ms: Option<u32>,
    error_rate: f32,
    consecutive_failures: u32,
}

fn get_health_json(state: &BridgeState) -> String {
    let tools = state.get_health_status();
    let server_healthy = state.health_checker.all_healthy();

    let response = HealthResponse {
        server_healthy,
        tools: tools.iter().map(|t| ToolHealthResponse {
            tool: t.tool.to_string(),
            healthy: t.healthy,
            last_check: t.last_check.to_rfc3339(),
            latency_ms: t.latency_ms,
            error_rate: t.error_rate,
            consecutive_failures: t.consecutive_failures,
        }).collect(),
    };

    serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string())
}

#[derive(Serialize)]
struct QuotasResponse {
    quotas: Vec<QuotaEntry>,
}

#[derive(Serialize)]
struct QuotaEntry {
    user_id: String,
    daily_used: u64,
    daily_limit: Option<u64>,
    monthly_used: u64,
    monthly_limit: Option<u64>,
}

fn get_quotas_json(_state: &BridgeState) -> String {
    // For now return empty - would need to expose quota internals
    let response = QuotasResponse { quotas: vec![] };
    serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string())
}

// Embedded HTML Dashboard
const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Polyglot Bridge Dashboard</title>
    <link rel="stylesheet" href="/style.css">
</head>
<body>
    <div class="container">
        <header>
            <h1>🌉 Polyglot Bridge Dashboard</h1>
            <div id="status" class="status"></div>
        </header>

        <div class="grid">
            <div class="card">
                <h2>📊 Metrics</h2>
                <div id="metrics">Loading...</div>
            </div>

            <div class="card">
                <h2>💚 Health Status</h2>
                <div id="health">Loading...</div>
            </div>

            <div class="card wide">
                <h2>🛠️ Tool Statistics</h2>
                <div id="tools">Loading...</div>
            </div>

            <div class="card">
                <h2>💾 Cache</h2>
                <div id="cache">Loading...</div>
            </div>

            <div class="card">
                <h2>⏱️ Uptime</h2>
                <div id="uptime">Loading...</div>
            </div>
        </div>

        <footer>
            <p>Polyglot-AI Bridge v<span id="version">-</span> | Auto-refresh: 5s</p>
        </footer>
    </div>
    <script src="/script.js"></script>
</body>
</html>"#;

const DASHBOARD_CSS: &str = r#"
* { box-sizing: border-box; margin: 0; padding: 0; }

body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
    color: #e0e0e0;
    min-height: 100vh;
    padding: 20px;
}

.container {
    max-width: 1200px;
    margin: 0 auto;
}

header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 30px;
    padding-bottom: 20px;
    border-bottom: 1px solid #333;
}

header h1 {
    font-size: 1.8rem;
    color: #fff;
}

.status {
    padding: 8px 16px;
    border-radius: 20px;
    font-size: 0.9rem;
    font-weight: 500;
}

.status.healthy { background: #28a745; color: #fff; }
.status.unhealthy { background: #dc3545; color: #fff; }

.grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
    gap: 20px;
}

.card {
    background: rgba(255, 255, 255, 0.05);
    border-radius: 12px;
    padding: 20px;
    backdrop-filter: blur(10px);
    border: 1px solid rgba(255, 255, 255, 0.1);
}

.card.wide {
    grid-column: span 2;
}

.card h2 {
    font-size: 1.1rem;
    margin-bottom: 15px;
    color: #fff;
    border-bottom: 1px solid rgba(255, 255, 255, 0.1);
    padding-bottom: 10px;
}

.metric {
    display: flex;
    justify-content: space-between;
    padding: 8px 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.05);
}

.metric:last-child { border-bottom: none; }

.metric-label { color: #aaa; }
.metric-value { font-weight: 600; color: #fff; }

.tool-row {
    display: grid;
    grid-template-columns: 1fr repeat(5, 80px);
    padding: 10px 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.05);
    font-size: 0.9rem;
}

.tool-row.header {
    font-weight: 600;
    color: #aaa;
    font-size: 0.8rem;
}

.tool-name { font-weight: 500; }

.health-badge {
    display: inline-block;
    padding: 4px 10px;
    border-radius: 12px;
    font-size: 0.8rem;
    font-weight: 500;
}

.health-badge.healthy { background: rgba(40, 167, 69, 0.2); color: #28a745; }
.health-badge.unhealthy { background: rgba(220, 53, 69, 0.2); color: #dc3545; }

.progress-bar {
    height: 8px;
    background: rgba(255, 255, 255, 0.1);
    border-radius: 4px;
    overflow: hidden;
    margin-top: 5px;
}

.progress-bar .fill {
    height: 100%;
    background: linear-gradient(90deg, #28a745, #20c997);
    border-radius: 4px;
    transition: width 0.3s ease;
}

footer {
    margin-top: 30px;
    text-align: center;
    color: #666;
    font-size: 0.85rem;
}

@media (max-width: 768px) {
    .card.wide { grid-column: span 1; }
    .tool-row { grid-template-columns: 1fr repeat(3, 60px); }
    .tool-row .hide-mobile { display: none; }
}
"#;

const DASHBOARD_JS: &str = r#"
async function fetchData(endpoint) {
    try {
        const response = await fetch(endpoint);
        return await response.json();
    } catch (e) {
        console.error('Fetch error:', e);
        return null;
    }
}

function formatUptime(seconds) {
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    const secs = seconds % 60;

    if (days > 0) return `${days}d ${hours}h ${mins}m`;
    if (hours > 0) return `${hours}h ${mins}m ${secs}s`;
    if (mins > 0) return `${mins}m ${secs}s`;
    return `${secs}s`;
}

function formatNumber(n) {
    if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
    if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
    return n.toString();
}

async function updateDashboard() {
    // Fetch all data
    const [status, metrics, health] = await Promise.all([
        fetchData('/api/status'),
        fetchData('/api/metrics'),
        fetchData('/api/health')
    ]);

    // Update status badge
    const statusEl = document.getElementById('status');
    if (health) {
        statusEl.textContent = health.server_healthy ? '✓ Healthy' : '✗ Unhealthy';
        statusEl.className = 'status ' + (health.server_healthy ? 'healthy' : 'unhealthy');
    }

    // Update version
    if (status) {
        document.getElementById('version').textContent = status.version;
    }

    // Update metrics
    if (metrics) {
        document.getElementById('metrics').innerHTML = `
            <div class="metric">
                <span class="metric-label">Active Connections</span>
                <span class="metric-value">${metrics.active_connections}</span>
            </div>
            <div class="metric">
                <span class="metric-label">Total Requests</span>
                <span class="metric-value">${formatNumber(metrics.total_requests)}</span>
            </div>
            <div class="metric">
                <span class="metric-label">Requests/min</span>
                <span class="metric-value">${metrics.requests_per_minute.toFixed(1)}</span>
            </div>
        `;

        document.getElementById('cache').innerHTML = `
            <div class="metric">
                <span class="metric-label">Entries</span>
                <span class="metric-value">${metrics.cache_entries}</span>
            </div>
            <div class="metric">
                <span class="metric-label">Hit Rate</span>
                <span class="metric-value">${(metrics.cache_hit_rate * 100).toFixed(1)}%</span>
            </div>
            <div class="progress-bar">
                <div class="fill" style="width: ${metrics.cache_hit_rate * 100}%"></div>
            </div>
            <div class="metric">
                <span class="metric-label">Memory</span>
                <span class="metric-value">${metrics.cache_memory_mb.toFixed(2)} MB</span>
            </div>
        `;

        document.getElementById('uptime').innerHTML = `
            <div style="font-size: 2rem; font-weight: 600; color: #fff; text-align: center; padding: 20px;">
                ${formatUptime(metrics.uptime_seconds)}
            </div>
        `;

        // Update tool stats
        let toolsHtml = `
            <div class="tool-row header">
                <span>Tool</span>
                <span>Total</span>
                <span>Success</span>
                <span class="hide-mobile">Failed</span>
                <span>Latency</span>
                <span class="hide-mobile">Status</span>
            </div>
        `;

        for (const tool of metrics.tool_stats) {
            const healthInfo = health?.tools?.find(t => t.tool === tool.tool);
            const isHealthy = healthInfo?.healthy ?? true;

            toolsHtml += `
                <div class="tool-row">
                    <span class="tool-name">${tool.tool}</span>
                    <span>${formatNumber(tool.total_requests)}</span>
                    <span>${formatNumber(tool.successful)}</span>
                    <span class="hide-mobile">${formatNumber(tool.failed)}</span>
                    <span>${tool.avg_latency_ms}ms</span>
                    <span class="hide-mobile">
                        <span class="health-badge ${isHealthy ? 'healthy' : 'unhealthy'}">
                            ${isHealthy ? 'OK' : 'DOWN'}
                        </span>
                    </span>
                </div>
            `;
        }

        document.getElementById('tools').innerHTML = toolsHtml;
    }

    // Update health
    if (health) {
        let healthHtml = '';
        for (const tool of health.tools) {
            healthHtml += `
                <div class="metric">
                    <span class="metric-label">${tool.tool}</span>
                    <span class="health-badge ${tool.healthy ? 'healthy' : 'unhealthy'}">
                        ${tool.healthy ? 'Healthy' : 'Unhealthy'}
                        ${tool.latency_ms ? ` (${tool.latency_ms}ms)` : ''}
                    </span>
                </div>
            `;
        }
        document.getElementById('health').innerHTML = healthHtml;
    }
}

// Initial load and auto-refresh
updateDashboard();
setInterval(updateDashboard, 5000);
"#;
