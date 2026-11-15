//! Static object catalog served inside the gateway's access and origin boundary.

use axum::{
    extract::Request,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use tower::ServiceExt;
use tower_http::services::ServeDir;

pub(crate) async fn serve(request: Request) -> Response {
    if request.uri().path() == "/catalog" {
        return axum::response::Redirect::temporary("/catalog/").into_response();
    }
    let root = match crate::status::sjel_root() {
        Ok(root) => root.join("apps/catalog/dist"),
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "catalog root is unavailable",
            )
                .into_response()
        }
    };
    serve_at(request, root).await
}

async fn serve_at(mut request: Request, root: std::path::PathBuf) -> Response {
    if !root.join("index.html").is_file() {
        return (StatusCode::SERVICE_UNAVAILABLE, "catalog is not built").into_response();
    }
    let path = request.uri().path().strip_prefix("/catalog").unwrap_or("/");
    let Ok(uri) = path.parse::<Uri>() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    *request.uri_mut() = uri;
    let mut response = match ServeDir::new(root).oneshot(request).await {
        Ok(response) => response.into_response(),
        Err(error) => match error {},
    };
    response.headers_mut().insert(header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'".parse().expect("static CSP"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static cache control"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff".parse().expect("static content policy"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "starts a fictional loopback server for bun run browser-check"]
    async fn catalog_browser_fixture() {
        use axum::{
            extract::Query,
            routing::{get, post},
            Json, Router,
        };
        use std::collections::HashMap;
        use std::future::IntoFuture;
        use std::sync::Arc;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/catalog/dist");
        assert!(root.join("index.html").is_file());
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../apps/catalog/fixtures/projects.json"))
                .unwrap();
        let fixture = Arc::new(fixture);
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let app = Router::new()
            .route(
                "/catalog/",
                get({
                    let root = root.clone();
                    move |request: Request| serve_at(request, root.clone())
                }),
            )
            .route(
                "/catalog/{*path}",
                get(move |request: Request| serve_at(request, root.clone())),
            )
            .route(
                "/projects/api/projects",
                get({
                    let fixture = fixture.clone();
                    move || {
                        let fixture = fixture.clone();
                        async move { Json(serde_json::json!({"projects": fixture["projects"]})) }
                    }
                }),
            )
            .route(
                "/projects/api/project",
                get({
                    let fixture = fixture.clone();
                    move |Query(query): Query<HashMap<String, String>>| {
                        let fixture = fixture.clone();
                        async move {
                            let mut records = fixture["records"].clone();
                            if query.get("content").is_some_and(|value| value == "false") {
                                for record in records.as_array_mut().unwrap() {
                                    record.as_object_mut().unwrap().remove("content");
                                }
                            }
                            Json(serde_json::json!({"records": records}))
                        }
                    }
                }),
            )
            .route(
                "/projects/api/record",
                get({
                    let fixture = fixture.clone();
                    move |Query(query): Query<HashMap<String, String>>| {
                        let fixture = fixture.clone();
                        async move {
                            let record =
                                fixture["records"]
                                    .as_array()
                                    .unwrap()
                                    .iter()
                                    .find(|record| {
                                        record["path"].as_str()
                                            == query.get("path").map(String::as_str)
                                    });
                            match record {
                                Some(record) => {
                                    Json(serde_json::json!({"record": record})).into_response()
                                }
                                None => StatusCode::NOT_FOUND.into_response(),
                            }
                        }
                    }
                }),
            )
            .route(
                "/__fixture/stop",
                post({
                    let shutdown = shutdown.clone();
                    move || {
                        let shutdown = shutdown.clone();
                        async move {
                            shutdown.notify_one();
                            StatusCode::NO_CONTENT
                        }
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        println!(
            "CATALOG_FIXTURE=http://{}/catalog/",
            listener.local_addr().unwrap()
        );
        let server = axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown.notified().await });
        tokio::time::timeout(std::time::Duration::from_secs(180), server.into_future())
            .await
            .expect("browser fixture shutdown timeout")
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires bun run build in apps/catalog"]
    async fn catalog_serves_built_assets_without_an_html_fallback_for_missing_files() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/catalog/dist");
        assert!(root.join("index.html").is_file());
        let request = Request::builder()
            .uri("/catalog/")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = serve_at(request, root.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("connect-src 'self'"));
        let request = Request::builder()
            .uri("/catalog/assets/missing.js")
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(
            serve_at(request, root).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
