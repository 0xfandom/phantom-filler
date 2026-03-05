//! HTTP request handlers for the REST API.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResponse};
use crate::state::AppState;

// ─── Health Endpoints ───────────────────────────────────────────────

/// GET /health — full system health report.
pub async fn health_report(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<phantom_metrics::health::HealthReport>>, ApiError> {
    let report = state.health.report();
    Ok(ApiResponse::ok(report))
}

/// GET /health/live — liveness probe (200 if server is running).
pub async fn health_live() -> Json<ApiResponse<LiveStatus>> {
    ApiResponse::ok(LiveStatus { alive: true })
}

/// GET /health/ready — readiness probe.
pub async fn health_ready(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<ReadyStatus>>, ApiError> {
    let ready = state.health.is_ready();
    let report = state.health.report();
    Ok(ApiResponse::ok(ReadyStatus {
        ready,
        status: report.status,
    }))
}

/// Liveness response body.
#[derive(Debug, Serialize)]
pub struct LiveStatus {
    pub alive: bool,
}

/// Readiness response body.
#[derive(Debug, Serialize)]
pub struct ReadyStatus {
    pub ready: bool,
    pub status: phantom_metrics::health::SystemStatus,
}

// ─── P&L Endpoints ──────────────────────────────────────────────────

/// GET /api/v1/pnl — P&L summary.
pub async fn pnl_summary(
    State(state): State<AppState>,
) -> Json<ApiResponse<phantom_inventory::pnl::PnlSummary>> {
    let summary = state.pnl.summary();
    ApiResponse::ok(summary)
}

/// GET /api/v1/pnl/daily — daily P&L history.
pub async fn pnl_daily(
    State(state): State<AppState>,
) -> Json<ApiResponse<Vec<phantom_inventory::pnl::DailyPnl>>> {
    let daily = state.pnl.all_daily_pnl();
    ApiResponse::ok(daily)
}

/// GET /api/v1/pnl/tokens — per-token P&L breakdown.
pub async fn pnl_tokens(
    State(state): State<AppState>,
) -> Json<ApiResponse<Vec<phantom_inventory::pnl::TokenPnl>>> {
    let tokens = state.pnl.all_token_pnl();
    ApiResponse::ok(tokens)
}

// ─── Fill Endpoints ─────────────────────────────────────────────────

/// Query parameters for fill listing.
#[derive(Debug, Deserialize)]
pub struct FillQuery {
    /// Maximum number of fills to return (default: 50, max: 500).
    pub limit: Option<usize>,
}

/// GET /api/v1/fills — recent fills.
pub async fn list_fills(
    State(state): State<AppState>,
    Query(query): Query<FillQuery>,
) -> Json<ApiResponse<Vec<phantom_inventory::pnl::FillRecord>>> {
    let limit = query.limit.unwrap_or(50).min(500);
    let fills = state.pnl.recent_fills(limit);
    ApiResponse::ok(fills)
}

/// GET /api/v1/fills/:id — get fill by ID.
pub async fn get_fill(
    State(state): State<AppState>,
    axum::extract::Path(fill_id): axum::extract::Path<String>,
) -> Result<Json<ApiResponse<phantom_inventory::pnl::FillRecord>>, ApiError> {
    match state.pnl.get_fill(&fill_id) {
        Some(record) => Ok(ApiResponse::ok(record)),
        None => Err(ApiError::NotFound(format!("fill {fill_id}"))),
    }
}

// ─── Risk Endpoints ─────────────────────────────────────────────────

/// Risk exposure response.
#[derive(Debug, Serialize)]
pub struct RiskStatus {
    /// Whether risk controls are enabled.
    pub enabled: bool,
    /// Current daily loss in wei.
    pub daily_loss_wei: u64,
    /// Number of pending (unconfirmed) fills.
    pub pending_fills: u32,
    /// Maximum single fill value (from config).
    pub max_single_fill_value: String,
    /// Daily loss limit in wei (from config).
    pub daily_loss_limit_wei: u64,
    /// Maximum concurrent pending fills allowed.
    pub max_pending_fills: u32,
}

/// GET /api/v1/risk — risk exposure summary.
pub async fn risk_status(State(state): State<AppState>) -> Json<ApiResponse<RiskStatus>> {
    let config = state.risk.config();
    let status = RiskStatus {
        enabled: config.enabled,
        daily_loss_wei: state.risk.daily_loss_wei(),
        pending_fills: state.risk.pending_count(),
        max_single_fill_value: config.max_single_fill_value.to_string(),
        daily_loss_limit_wei: config.daily_loss_limit_wei,
        max_pending_fills: config.max_pending_fills,
    };
    ApiResponse::ok(status)
}

// ─── System Endpoints ───────────────────────────────────────────────

/// System status response.
#[derive(Debug, Serialize)]
pub struct SystemInfo {
    /// Application name.
    pub name: &'static str,
    /// Version string.
    pub version: &'static str,
}

/// GET /api/v1/status — system information.
pub async fn system_status() -> Json<ApiResponse<SystemInfo>> {
    ApiResponse::ok(SystemInfo {
        name: "phantom-filler",
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_query_default_limit() {
        let query: FillQuery = serde_json::from_str("{}").expect("deserialize");
        assert!(query.limit.is_none());
    }

    #[test]
    fn fill_query_with_limit() {
        let query: FillQuery = serde_json::from_str(r#"{"limit": 100}"#).expect("deserialize");
        assert_eq!(query.limit, Some(100));
    }

    #[test]
    fn system_info_serialization() {
        let info = SystemInfo {
            name: "phantom-filler",
            version: "0.1.0",
        };
        let json = serde_json::to_string(&info).expect("serialize");
        assert!(json.contains("phantom-filler"));
    }

    #[test]
    fn risk_status_serialization() {
        let status = RiskStatus {
            enabled: true,
            daily_loss_wei: 1000,
            pending_fills: 2,
            max_single_fill_value: "1000000".to_string(),
            daily_loss_limit_wei: 5000,
            max_pending_fills: 10,
        };
        let json = serde_json::to_string(&status).expect("serialize");
        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"pending_fills\":2"));
    }
}
