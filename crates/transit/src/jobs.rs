//! Local journey and session jobs shared by Transit and Scouting.

use std::{borrow::Cow, path::Path, time::Duration};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sjel_server::{owner_snapshot, InboundAuth};

use crate::{
    config::owner_path,
    store::{CandidateDest, SessionRow, TransitStore, TripWithLegs},
    travel::Journey,
};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize, Serialize)]
#[serde(tag = "job", rename_all = "snake_case")]
enum Job<'a> {
    Record {
        journey: Box<Cow<'a, Journey>>,
        #[serde(borrow)]
        origin: Cow<'a, str>,
        #[serde(borrow)]
        destination: Cow<'a, str>,
        #[serde(borrow)]
        reason: Cow<'a, str>,
        #[serde(borrow)]
        session: Option<Cow<'a, str>>,
    },
    Session {
        #[serde(borrow)]
        id: Cow<'a, str>,
    },
    SessionTrips {
        #[serde(borrow)]
        id: Cow<'a, str>,
    },
    UpsertSession {
        #[serde(borrow)]
        id: Cow<'a, str>,
        #[serde(borrow)]
        origin: Cow<'a, str>,
        #[serde(borrow)]
        intent: Cow<'a, str>,
        candidates: Cow<'a, [CandidateDest]>,
        #[serde(borrow)]
        start: Cow<'a, str>,
        #[serde(borrow)]
        end: Cow<'a, str>,
    },
}

impl Job<'_> {
    fn run(self, store: &TransitStore) -> Fallible<serde_json::Value> {
        Ok(match self {
            Self::Record {
                journey,
                origin,
                destination,
                reason,
                session,
            } => serde_json::to_value(store.record_journey(
                &journey,
                &origin,
                &destination,
                &reason,
                session.as_deref(),
            )?)?,
            Self::Session { id } => serde_json::to_value(store.get_session(&id)?)?,
            Self::SessionTrips { id } => serde_json::to_value(store.list_session_trips(&id)?)?,
            Self::UpsertSession {
                id,
                origin,
                intent,
                candidates,
                start,
                end,
            } => serde_json::to_value(store.upsert_session(
                &id,
                &origin,
                &intent,
                &candidates,
                &start,
                &end,
            )?)?,
        })
    }
}

pub enum Client {
    Owner(reqwest::blocking::Client),
    Sqlite(TransitStore),
}

impl Client {
    pub fn open(database: &Path) -> Fallible<Self> {
        if owner_path().is_some() || cfg!(feature = "turso-wal") {
            Ok(Self::Owner(socket_client()?))
        } else {
            Ok(Self::Sqlite(TransitStore::open(database)?))
        }
    }

    fn run<T: DeserializeOwned>(&self, job: Job<'_>) -> Fallible<T> {
        match self {
            Self::Sqlite(store) => Ok(serde_json::from_value(job.run(store)?)?),
            Self::Owner(client) => {
                let url = "http://127.0.0.1/journey-jobs";
                let send =
                    || InboundAuth::with_loopback_auth(client.post(url).json(&job), url).send();
                let mut response = send();
                if response.as_ref().is_err_and(reqwest::Error::is_connect) {
                    start_owner()?;
                    response = send();
                }
                Ok(response?.error_for_status()?.json()?)
            }
        }
    }

    pub fn record_journey(
        &self,
        journey: &Journey,
        origin: &str,
        destination: &str,
        reason: &str,
        session: Option<&str>,
    ) -> Fallible<bool> {
        self.run(Job::Record {
            journey: Box::new(Cow::Borrowed(journey)),
            origin: origin.into(),
            destination: destination.into(),
            reason: reason.into(),
            session: session.map(Cow::Borrowed),
        })
    }

    pub fn get_session(&self, id: &str) -> Fallible<Option<SessionRow>> {
        self.run(Job::Session { id: id.into() })
    }

    pub fn list_session_trips(&self, id: &str) -> Fallible<Vec<TripWithLegs>> {
        self.run(Job::SessionTrips { id: id.into() })
    }

    pub fn upsert_session(
        &self,
        id: &str,
        origin: &str,
        intent: &str,
        candidates: &[CandidateDest],
        start: &str,
        end: &str,
    ) -> Fallible<bool> {
        self.run(Job::UpsertSession {
            id: id.into(),
            origin: origin.into(),
            intent: intent.into(),
            candidates: Cow::Borrowed(candidates),
            start: start.into(),
            end: end.into(),
        })
    }
}

fn socket_client() -> Fallible<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .unix_socket(owner_snapshot::socket_path("transit")?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?)
}

fn start_owner() -> Fallible<()> {
    let port = sjel_config::env_var("SJEL_STATUS_PORT")
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
        .unwrap_or(8082);
    let url = format!("http://127.0.0.1:{port}/api/sjel-status/capabilities/transit/start");
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?;
    InboundAuth::with_loopback_auth(client.post(&url), &url)
        .send()?
        .error_for_status()?;
    Ok(())
}

#[cfg(feature = "turso-wal")]
pub fn snapshot(database: &Path) -> Fallible<()> {
    let url = "http://127.0.0.1/snapshot";
    let client = socket_client()?;
    match InboundAuth::with_loopback_auth(client.post(url), url).send() {
        Ok(response) => {
            response.error_for_status()?;
            Ok(())
        }
        Err(error) if error.is_connect() => {
            let target = sjel_config::overlay_data_dir("transit")
                .ok_or("No active Sjel overlay")?
                .join("snapshots/transit.db");
            TransitStore::open(database)?.snapshot(database, &target)
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(feature = "turso-wal")]
pub async fn serve(database: std::sync::Arc<std::path::PathBuf>) {
    use axum::{body::Bytes, http::StatusCode, routing::post, Json, Router};

    let jobs_database = database.clone();
    let jobs = Router::new().route(
        "/journey-jobs",
        post(move |body: Bytes| async move {
            tokio::task::spawn_blocking(move || {
                let job: Job<'_> = serde_json::from_slice(&body)
                    .map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
                TransitStore::open(&jobs_database)
                    .and_then(|store| job.run(&store))
                    .map(Json)
                    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
            })
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        }),
    );
    owner_snapshot::serve_with_jobs(
        "transit",
        database,
        |path, target| {
            TransitStore::open(path)
                .and_then(|store| store.snapshot(path, target))
                .map_err(|error| error.to_string())
        },
        jobs,
    )
    .await;
}
