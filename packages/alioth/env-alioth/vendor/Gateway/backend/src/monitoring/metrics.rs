use prometheus::{CounterVec, HistogramOpts, HistogramVec, Opts, Registry};
use std::time::Instant;

const LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

pub struct Metrics {
    pub requests_total: CounterVec,
    pub request_duration_seconds: HistogramVec,
    pub errors_total: CounterVec,
    pub registry: Registry,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();

        let requests_total = CounterVec::new(
            Opts::new("http_requests_total", "Total HTTP requests").const_labels(
                vec![("service".to_string(), "gateway".to_string())]
                    .into_iter()
                    .collect(),
            ),
            &["method", "route", "status"],
        )
        .expect("Failed to create requests_total counter");

        let request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "http_request_duration_seconds",
                "Request duration in seconds",
            )
            .buckets(LATENCY_BUCKETS.to_vec())
            .const_labels(
                vec![("service".to_string(), "gateway".to_string())]
                    .into_iter()
                    .collect(),
            ),
            &["method", "route", "status"],
        )
        .expect("Failed to create request_duration histogram");

        let errors_total = CounterVec::new(
            Opts::new("http_errors_total", "Total HTTP errors").const_labels(
                vec![("service".to_string(), "gateway".to_string())]
                    .into_iter()
                    .collect(),
            ),
            &["method", "route", "status"],
        )
        .expect("Failed to create errors_total counter");

        registry
            .register(Box::new(requests_total.clone()))
            .expect("Failed to register requests_total");
        registry
            .register(Box::new(request_duration_seconds.clone()))
            .expect("Failed to register request_duration");
        registry
            .register(Box::new(errors_total.clone()))
            .expect("Failed to register errors_total");

        Metrics {
            requests_total,
            request_duration_seconds,
            errors_total,
            registry,
        }
    }

    pub fn inc_requests(&self, method: &str, route: &str, status: u16) {
        self.requests_total
            .with_label_values(&[method, route, &status.to_string()])
            .inc();
    }

    pub fn inc_errors(&self, method: &str, route: &str, status: u16) {
        self.errors_total
            .with_label_values(&[method, route, &status.to_string()])
            .inc();
    }

    pub fn observe_duration(&self, duration: f64, method: &str, route: &str, status: u16) {
        self.request_duration_seconds
            .with_label_values(&[method, route, &status.to_string()])
            .observe(duration);
    }
}

pub struct RequestTimer {
    start: Instant,
    method: String,
    route: String,
    status: u16,
    metrics: std::sync::Arc<Metrics>,
}

impl RequestTimer {
    pub fn new(
        method: String,
        route: String,
        status: u16,
        metrics: std::sync::Arc<Metrics>,
    ) -> Self {
        RequestTimer {
            start: Instant::now(),
            method,
            route,
            status,
            metrics,
        }
    }
}

impl Drop for RequestTimer {
    fn drop(&mut self) {
        let duration = self.start.elapsed().as_secs_f64();
        self.metrics
            .observe_duration(duration, &self.method, &self.route, self.status);
        self.metrics
            .inc_requests(&self.method, &self.route, self.status);
        if self.status >= 400 {
            self.metrics
                .inc_errors(&self.method, &self.route, self.status);
        }
    }
}
