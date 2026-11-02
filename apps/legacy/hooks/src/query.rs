//! Fallible queries retain their last successful value and expose failures for retry.

use std::future::Future;
use std::ops::Deref;

use dioxus::prelude::*;

pub struct Query<T: 'static> {
    pub result: Resource<Result<T, api::ApiError>>,
    data: Signal<Option<T>>,
}

impl<T> Copy for Query<T> {}

impl<T> Clone for Query<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Deref for Query<T> {
    type Target = Signal<Option<T>>;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> Query<T> {
    pub fn error(&self) -> Option<api::ApiError> {
        self.result
            .read()
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .cloned()
    }

    pub fn restart(&mut self) {
        self.result.restart();
    }
}

pub fn use_query<T: Clone + 'static, F: Future<Output = Result<T, api::ApiError>> + 'static>(
    mut fetch: impl FnMut() -> F + 'static,
) -> Query<T> {
    let mut data = use_signal(|| None);
    let result = use_resource(move || {
        let pending = fetch();
        async move {
            let result = pending.await;
            match &result {
                Ok(value) => data.set(Some(value.clone())),
                Err(error) => {
                    tracing::warn!(%error, "query failed");
                    crate::toast::toast_error(&error.to_string());
                }
            }
            result
        }
    });
    Query { result, data }
}
