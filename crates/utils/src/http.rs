//! Every HTTP client starts here, because reqwest prefers native-tls whenever any crate compiles it in.
#![allow(clippy::disallowed_methods)]

/// A client builder on rustls.
pub fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().tls_backend_rustls()
}

/// A rustls client with reqwest's defaults; panics where `reqwest::Client::new` would.
pub fn client() -> reqwest::Client {
    match builder().build() {
        Ok(client) => client,
        Err(error) => panic!("the TLS backend failed to initialize: {error}"),
    }
}

/// A blocking client builder on rustls.
pub fn blocking_builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder().tls_backend_rustls()
}

/// A client builder on the platform's TLS (OpenSSL on Linux), for servers that reset rustls's handshake.
#[cfg(not(target_os = "android"))]
pub fn native_tls_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().tls_backend_native()
}
