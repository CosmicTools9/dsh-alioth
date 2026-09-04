use actix_web::{
    body::EitherBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error,
};
use futures::future::{ok, LocalBoxFuture, Ready};
use std::{rc::Rc, sync::Arc, time::Instant};

use crate::monitoring::{sanitize_path, Metrics};

pub struct MetricsMiddleware;

impl<S, B> Transform<S, ServiceRequest> for MetricsMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Transform = MetricsMiddlewareService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(MetricsMiddlewareService {
            service: Rc::new(service),
        })
    }
}

pub struct MetricsMiddlewareService<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for MetricsMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.service.poll_ready(cx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let start = Instant::now();

        Box::pin(async move {
            let route = req
                .match_pattern()
                .unwrap_or_else(|| req.uri().path().to_string());
            let method = req.method().as_str().to_string();

            let metrics = req
                .app_data::<web::Data<Arc<Metrics>>>()
                .map(|m| m.get_ref().clone());

            let res = service.call(req).await?;

            let elapsed = start.elapsed();
            let status = res.status().as_u16();

            if let Some(m) = metrics {
                let sanitized_route = sanitize_path(&route);
                m.observe_duration(elapsed.as_secs_f64(), &method, &sanitized_route, status);
                m.inc_requests(&method, &sanitized_route, status);
                if status >= 400 {
                    m.inc_errors(&method, &sanitized_route, status);
                }
            }

            Ok(res.map_into_left_body())
        })
    }
}

use actix_web::web;
