//! Locale middleware for Actix-web.
//!
//! Extracts the preferred locale from the `Accept-Language` header
//! (or JWT claim / cookie in future iterations) and injects it into
//! request extensions as `Locale`. Also sets the `Content-Language`
//! response header.

use actix_web::{
    body::EitherBody,
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    http::header::HeaderName,
    Error, HttpMessage,
};
use futures::future::{ok, LocalBoxFuture, Ready};
use i18n::Locale;
use std::rc::Rc;
use std::task::{Context, Poll};

const ACCEPT_LANGUAGE: &str = "accept-language";

/// Middleware that resolves and injects the request locale.
#[derive(Clone, Debug, Default)]
pub struct LocaleMiddleware;

impl LocaleMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl<S, B> Transform<S, ServiceRequest> for LocaleMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type InitError = ();
    type Transform = LocaleMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(LocaleMiddlewareService {
            service: Rc::new(service),
        })
    }
}

pub struct LocaleMiddlewareService<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for LocaleMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<EitherBody<B>>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let locale = resolve_locale_from_request(&req);
        req.extensions_mut().insert(locale.clone());

        let fut = self.service.call(req);

        Box::pin(async move {
            let mut res = fut.await?;
            res.response_mut().headers_mut().insert(
                HeaderName::from_static("content-language"),
                locale.as_str().parse().unwrap(),
            );
            Ok(res.map_into_left_body())
        })
    }
}

fn resolve_locale_from_request(req: &ServiceRequest) -> Locale {
    let default = Locale::ZH_CN;

    if let Some(header_value) = req.headers().get(ACCEPT_LANGUAGE) {
        if let Ok(header_str) = header_value.to_str() {
            let supported: Vec<&str> = super::supported_locales()
                .iter()
                .map(|s| s.as_str())
                .collect();
            return i18n::resolve_locale(header_str, &supported, default);
        }
    }

    Locale::new(default)
}
