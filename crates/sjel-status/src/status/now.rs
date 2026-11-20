use super::*;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Active or upcoming trip for the traveler context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NowTripInfo {
    pub id: String,
    pub title: String,
    pub date_start: String,
    pub date_end: String,
    pub destinations: String,
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn query_trips(db_path: &Path) -> Option<(Option<NowTripInfo>, Option<NowTripInfo>)> {
    let pool = sjel_store::open_pool(db_path, "trips", |_| Ok(())).ok()?;
    let conn = pool.get().ok()?;

    let table_exists: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='trips_plans'",
            [],
            |row| Ok(row.get::<_, i64>(0)? > 0),
        )
        .unwrap_or(false);
    if !table_exists {
        return None;
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, title, date_start, date_end, destinations
               FROM trips_plans
              WHERE status != 'archived'
                AND date_end >= date('now', 'localtime')
              ORDER BY date_start ASC
              LIMIT 5",
        )
        .ok()?;

    let rows = stmt
        .query_map([], |row| {
            Ok(NowTripInfo {
                id: row.get(0)?,
                title: row.get(1)?,
                date_start: row.get(2)?,
                date_end: row.get(3)?,
                destinations: row.get(4)?,
            })
        })
        .ok()?;

    let trips: Vec<NowTripInfo> = rows.filter_map(Result::ok).collect();
    let today_str: String = conn
        .query_row("SELECT date('now', 'localtime')", [], |row| row.get(0))
        .unwrap_or_default();

    let active = trips
        .iter()
        .find(|t| t.date_start <= today_str && t.date_end >= today_str)
        .cloned();

    let next = trips.iter().find(|t| t.date_start > today_str).cloned();

    Some((active, next))
}

/// `GET /api/sjel-status/now` — single aggregated "Now" endpoint for native menu bar and Home.
///
/// Combines systems health, active/upcoming calendar commitments, active trip legs,
/// and pending agent approvals in one lightweight roundtrip.
pub(crate) async fn now_handler() -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let db_path = sjel_config::database_path();

    // 1. Systems Health
    let (healthy, unhealthy_services) = match sjel_status_health_handler().await {
        Ok(Json(health)) => {
            let unhealthy: Vec<String> = health
                .capabilities
                .into_iter()
                .filter(|(_, c)| !c.up)
                .map(|(name, _)| name)
                .collect();
            (health.ok, unhealthy)
        }
        Err(_) => (false, Vec::new()),
    };

    // 2. Pending Approvals
    let pending_approvals_count = match agent_handler().await {
        Ok(Json(agent_val)) => agent_val["pending"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
        Err(_) => 0,
    };

    // 3. Calendar Ahead & Current
    let calendar = match registry().await {
        Ok(services) => super::search::owner_get::<calendar_api::NowEntries>(
            &services,
            "calendar",
            calendar_api::ROUTE_NOW,
            &[],
        )
        .await
        .unwrap_or_default(),
        Err(_) => calendar_api::NowEntries::default(),
    };

    // 4. Trips
    let (active_trip, next_trip) = query_trips(&db_path).unwrap_or((None, None));

    // 5. Warnings
    let mut warnings = Vec::new();
    if !healthy && !unhealthy_services.is_empty() {
        warnings.push(format!("Services down: {}", unhealthy_services.join(", ")));
    }
    if pending_approvals_count > 0 {
        warnings.push(format!(
            "{pending_approvals_count} agent action(s) require approval"
        ));
    }

    Ok(Json(json!({
        "timestamp_epoch": now_epoch_secs(),
        "healthy": healthy,
        "unhealthy_services": unhealthy_services,
        "current_event": calendar.current,
        "next_event": calendar.next,
        "calendar_ahead": calendar.ahead,
        "active_trip": active_trip,
        "next_trip": next_trip,
        "pending_approvals_count": pending_approvals_count,
        "warnings": warnings,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn now_handler_answers_valid_shape_without_crashing() {
        let res = now_handler().await;
        assert!(
            res.is_ok(),
            "now_handler should never fail: {:?}",
            res.err()
        );
        let Json(val) = res.unwrap();
        assert!(val.get("timestamp_epoch").is_some());
        assert!(val.get("healthy").is_some());
        assert!(val.get("unhealthy_services").is_some());
        assert!(val.get("calendar_ahead").is_some());
        assert!(val.get("warnings").is_some());
    }
}
