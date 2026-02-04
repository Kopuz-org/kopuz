use components::settings_items::{DirectoryPicker, ServerSettings, SettingItem, ThemeSelector};
use components::settings_popups::{AddServerPopup, LoginPopup};
use config::AppConfig;
use dioxus::prelude::*;
use server::jellyfin::JellyfinRemote;

#[component]
pub fn Settings(config: Signal<AppConfig>) -> Element {
    let mut show_add_server = use_signal(|| false);
    let mut show_login = use_signal(|| false);

    let mut server_name = use_signal(|| String::new());
    let mut server_url = use_signal(|| String::new());

    let mut username = use_signal(|| String::new());
    let mut password = use_signal(|| String::new());

    let mut error = use_signal(|| Option::<String>::None);
    let mut login_error = use_signal(|| Option::<String>::None);
    let mut is_loading = use_signal(|| false);

    let handle_add_server = move |_| {
        if !server_url().starts_with("http") {
            error.set(Some("Invalid server URL".into()));
            return;
        }

        let new_server = config::JellyfinServer {
            name: if server_name().is_empty() {
                "Local Jellyfin".into()
            } else {
                server_name()
            },
            url: server_url(),
            access_token: None,
        };

        config.write().server = Some(new_server);

        server_name.set(String::new());
        server_url.set(String::new());
        error.set(None);
        show_add_server.set(false);

        show_login.set(true);
    };

    let handle_login = move |_| {
        if username().is_empty() || password().is_empty() {
            login_error.set(Some("Username and password are required".into()));
            return;
        }

        if let Some(server) = &config.read().server {
            let server_url = server.url.clone();
            let device_id = config.read().device_id.clone();
            let user = username();
            let pass = password();

            is_loading.set(true);
            login_error.set(None);

            spawn(async move {
                let mut remote = JellyfinRemote::new(&server_url, None, &device_id);
                let result = remote.login(&user, &pass).await;

                is_loading.set(false);

                match result {
                    Ok(token) => {
                        if let Some(server) = config.write().server.as_mut() {
                            server.access_token = Some(token);
                        }
                        username.set(String::new());
                        password.set(String::new());
                        login_error.set(None);
                        show_login.set(false);
                    }
                    Err(e) => {
                        login_error.set(Some(format!("Login failed: {}", e)));
                    }
                }
            });
        }
    };

    rsx! {
        div { class: "p-8 max-w-4xl",
            h1 { class: "text-3xl font-bold text-white mb-6", "Settings" }

            div { class: "space-y-8",
                section {
                    h2 {
                        class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                        "General"
                    }

                    div { class: "space-y-4",
                        SettingItem {
                            title: "Appearance",
                            description: "Select your preferred color theme.".to_string(),
                            control: rsx! {
                                ThemeSelector {
                                    current_theme: config.read().theme.clone(),
                                    on_change: move |theme| {
                                        config.write().theme = theme;
                                    }
                                }
                            }
                        }

                        SettingItem {
                            title: "Music Directory",
                            description: format!("Current path: {}", config.read().music_directory.display()),
                            control: rsx! {
                                DirectoryPicker {
                                    on_change: move |path| {
                                        config.write().music_directory = path;
                                    }
                                }
                            }
                        }

                        SettingItem {
                            title: "Jellyfin Server",
                            description: if config.read().server.is_some() {
                                "Server configured".to_string()
                            } else {
                                "No server configured".to_string()
                            },
                            control: rsx! {
                                ServerSettings {
                                    server: config.read().server.clone(),
                                    on_add: move |_| show_add_server.set(true),
                                    on_delete: move |_| config.write().server = None,
                                    on_login: move |_| show_login.set(true),
                                }
                            }
                        }
                    }
                }

                if show_add_server() {
                    AddServerPopup {
                        server_name,
                        server_url,
                        error,
                        on_close: move |_| show_add_server.set(false),
                        on_save: handle_add_server
                    }
                }

                if show_login() {
                    LoginPopup {
                        username,
                        password,
                        error: login_error,
                        loading: is_loading,
                        on_close: move |_| {
                            show_login.set(false);
                            username.set(String::new());
                            password.set(String::new());
                            login_error.set(None);
                        },
                        on_save: handle_login
                    }
                }
            }
        }
    }
}
