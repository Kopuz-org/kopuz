/*
 * Stand-in for the BuildConfig the upstream rustls-platform-verifier gradle
 * module generates via buildConfigField. Kopuz compiles the vendored
 * CertificateVerifier.kt inside the app module, where no such class exists;
 * `TEST` gates test-only mock-root hooks and is permanently off in the app.
 */
package org.rustls.platformverifier

internal object BuildConfig {
    const val TEST = false
}
