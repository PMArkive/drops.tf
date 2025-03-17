use axum::body::Body;
use axum::extract::{connect_info, MatchedPath};
use axum::http::Request;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{middleware, Extension, Router};
use dropstf::{
    api_search, get_log, handler_404, last_log, page_player, page_top_stats, DataSource, TopOrder,
};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server;
use main_error::MainError;
use metrics::{counter, histogram};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{SpanExporter, WithExportConfig, WithTonicConfig};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use sqlx::postgres::PgPool;
use std::convert::Infallible;
use std::fs::{set_permissions, Permissions};
use std::future::ready;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tokio::net::unix::UCred;
use tokio::net::{UnixListener, UnixStream};
use tokio::time::Instant;
use tower_http::trace::TraceLayer;
use tower_service::Service;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

enum Listen {
    Port(u16),
    Socket(String),
}

#[tokio::main]
async fn main() -> Result<(), MainError> {
    if let Ok(tracing_endpoint) = dotenvy::var("TRACING_ENDPOINT") {
        let tls_config = tonic::transport::ClientTlsConfig::new().with_native_roots();
        let otlp_exporter = SpanExporter::builder()
            .with_tonic()
            .with_endpoint(tracing_endpoint)
            .with_tls_config(tls_config);
        let tracer = SdkTracerProvider::builder()
            .with_resource(
                Resource::builder()
                    .with_attribute(KeyValue::new("service.name", "drops.tf"))
                    .build(),
            )
            .with_batch_exporter(otlp_exporter.build()?)
            .build()
            .tracer("drops.tf");
        let open_telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(
                std::env::var("RUST_LOG")
                    .unwrap_or_else(|_| "dropstf=debug,tower_http=debug,sqlx=debug".into()),
            ))
            .with(open_telemetry)
            .with(
                tracing_subscriber::fmt::layer().with_filter(tracing_subscriber::EnvFilter::new(
                    std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()),
                )),
            )
            .try_init()?;
    }

    let database_url = dotenvy::var("DATABASE_URL")?;
    let api_key = dotenvy::var("STEAM_API_KEY")?;
    let listen = match dotenvy::var("SOCKET") {
        Ok(socket) => Listen::Socket(socket),
        _ => Listen::Port(u16::from_str(&dotenvy::var("PORT")?)?),
    };

    let pool = PgPool::connect(&database_url).await?;
    let data_source = DataSource::new(pool, api_key);

    let recorder_handle = setup_metrics_recorder();

    let app = Router::new()
        .route(
            "/",
            get(|data_source| page_top_stats(data_source, TopOrder::Drops)),
        )
        .route(
            "/dpg",
            get(|data_source| page_top_stats(data_source, TopOrder::Dpg)),
        )
        .route(
            "/dph",
            get(|data_source| page_top_stats(data_source, TopOrder::Dps)),
        )
        .route(
            "/dpu",
            get(|data_source| page_top_stats(data_source, TopOrder::Dpu)),
        )
        .route("/profile/{steam_id}", get(page_player))
        .route("/search", get(api_search))
        .route("/metrics", get(move || ready(recorder_handle.render())))
        .route("/api/log/last", get(last_log))
        .route("/api/log/{id}", get(get_log))
        .route_layer(middleware::from_fn(track_metrics))
        .layer(Extension(data_source))
        .layer(TraceLayer::new_for_http())
        .fallback(handler_404);

    match listen {
        Listen::Port(port) => {
            let addr = SocketAddr::from(([0, 0, 0, 0], port));
            tracing::info!("listening on {}", addr);
            let listener = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(listener, app).await?;
        }
        Listen::Socket(socket) => {
            tracing::info!("listening on {}", socket);
            let socket_path: PathBuf = socket.into();
            if socket_path.exists() {
                std::fs::remove_file(&socket_path)?;
            }
            let listener = UnixListener::bind(&socket_path)?;
            set_permissions(&socket_path, Permissions::from_mode(0o666))?;

            let mut make_service = app.into_make_service_with_connect_info::<UdsConnectInfo>();

            // See https://github.com/tokio-rs/axum/blob/main/examples/serve-with-hyper/src/main.rs for
            // more details about this setup
            loop {
                let (socket, _remote_addr) = listener.accept().await?;

                let tower_service = unwrap_infallible(make_service.call(&socket).await);

                tokio::spawn(async move {
                    let socket = TokioIo::new(socket);

                    let hyper_service =
                        hyper::service::service_fn(move |request: Request<Incoming>| {
                            tower_service.clone().call(request)
                        });

                    if let Err(err) = server::conn::auto::Builder::new(TokioExecutor::new())
                        .serve_connection_with_upgrades(socket, hyper_service)
                        .await
                    {
                        eprintln!("failed to serve connection: {err:#}");
                    }
                });
            }
        }
    }

    Ok(())
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct UdsConnectInfo {
    peer_addr: Arc<tokio::net::unix::SocketAddr>,
    peer_cred: UCred,
}

impl connect_info::Connected<&UnixStream> for UdsConnectInfo {
    fn connect_info(target: &UnixStream) -> Self {
        let peer_addr = target.peer_addr().unwrap();
        let peer_cred = target.peer_cred().unwrap();

        Self {
            peer_addr: Arc::new(peer_addr),
            peer_cred,
        }
    }
}

fn unwrap_infallible<T>(result: Result<T, Infallible>) -> T {
    match result {
        Ok(value) => value,
        Err(err) => match err {},
    }
}

fn setup_metrics_recorder() -> PrometheusHandle {
    const EXPONENTIAL_SECONDS: &[f64] = &[
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
    ];

    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("http_requests_duration_seconds".to_string()),
            EXPONENTIAL_SECONDS,
        )
        .unwrap()
        .install_recorder()
        .unwrap()
}

async fn track_metrics(req: Request<Body>, next: Next) -> impl IntoResponse {
    let start = Instant::now();
    let path = if let Some(matched_path) = req.extensions().get::<MatchedPath>() {
        matched_path.as_str().to_owned()
    } else {
        req.uri().path().to_owned()
    };
    let method = req.method().clone();

    let response = next.run(req).await;

    let latency = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    if path != "/metrics" {
        let labels = [
            ("method", method.to_string()),
            ("path", path),
            ("status", status),
        ];

        counter!("http_requests_total", &labels).increment(1);
        histogram!("http_requests_duration_seconds", &labels).record(latency);
    }

    response
}
