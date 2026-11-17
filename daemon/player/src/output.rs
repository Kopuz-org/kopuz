//! Which output device the sink opens, and the list a frontend picks one from.

use std::str::FromStr;

use cpal::traits::{DeviceTrait, HostTrait};

/// An output the sink can open. `id` is a cpal [`cpal::DeviceId`] in its
/// `host:device` form, which is what `Playback::output_device` stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    /// Its card's raw `hw:` device is listed, so exclusive output can hold it.
    pub exclusive: bool,
}

/// Every output the default host offers. On ALSA this includes the raw
/// `hw:` and `plughw:` devices, so a card PipeWire has released (`pw-reserve`)
/// can be opened directly.
pub fn output_devices() -> Vec<OutputDevice> {
    let host = cpal::default_host();
    let default = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let devices = match host.output_devices() {
        Ok(devices) => devices,
        Err(error) => {
            tracing::warn!(%error, "failed to enumerate output devices");
            return Vec::new();
        }
    };
    let listed = devices
        .filter_map(|device| {
            let id = device.id().ok()?;
            let name = device
                .description()
                // ALSA leaves "card, " when a PCM has no name of its own.
                .map(|description| description.name().trim_end_matches([',', ' ']).to_string())
                .unwrap_or_else(|_| id.id().to_string());
            Some(Listed {
                id: id.to_string(),
                backend_id: id.id().to_string(),
                name,
            })
        })
        .collect();
    labelled(listed, default.as_deref())
}

struct Listed {
    id: String,
    backend_id: String,
    name: String,
}

/// ALSA's `null` PCM, which throws every sample away; a stored choice of it still opens, it is just never offered.
const SILENT: &str = "alsa:null";

/// ALSA names a card's `hw:`, `plughw:` and `front:` paths alike, so a shared name gets its device path.
fn labelled(mut listed: Vec<Listed>, default: Option<&str>) -> Vec<OutputDevice> {
    listed.retain(|device| device.id != SILENT);
    let ids: std::collections::HashSet<String> =
        listed.iter().map(|device| device.id.clone()).collect();
    let mut seen = std::collections::HashSet::new();
    let shared: std::collections::HashSet<String> = listed
        .iter()
        .filter(|device| !seen.insert(device.name.as_str()))
        .map(|device| device.name.clone())
        .collect();
    listed
        .into_iter()
        .map(|device| OutputDevice {
            is_default: default == Some(device.id.as_str()),
            exclusive: exclusive_target(&device.id).is_some_and(|target| ids.contains(&target)),
            name: if shared.contains(&device.name) {
                format!("{} ({})", device.name, device.backend_id)
            } else {
                device.name
            },
            id: device.id,
        })
        .collect()
}

/// How the sink came by the device it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pick {
    /// The device the setting names.
    Chosen,
    /// No device is set, so the system default.
    Default,
    /// The named device is gone or unreadable; the system default stands in.
    Fallback,
    /// The raw hardware device behind the named one, held alone.
    Exclusive,
}

impl Pick {
    /// Whether a change of the system default should move the stream.
    pub(crate) fn follows_default(self) -> bool {
        matches!(self, Self::Default | Self::Fallback)
    }
}

/// The ALSA `hw:` device id behind a card-level id (`plughw:`, `front:`, `sysdefault:`, `dmix:`), which no other client can share.
pub(crate) fn exclusive_target(id: &str) -> Option<String> {
    let pcm = id.strip_prefix("alsa:")?;
    let (prefix, args) = pcm.split_once(':')?;
    if !matches!(prefix, "hw" | "plughw" | "front" | "sysdefault" | "dmix") {
        return None;
    }
    let (mut card, mut dev) = (None, None);
    for (index, part) in args.split(',').map(str::trim).enumerate() {
        match (part.split_once('='), index) {
            (Some(("CARD", value)), _) => card = Some(value),
            (Some(("DEV", value)), _) => dev = Some(value),
            (None, 0) => card = Some(part),
            (None, 1) => dev = Some(part),
            _ => return None,
        }
    }
    let card = card.filter(|card| !card.is_empty())?;
    let dev = dev.unwrap_or("0");
    dev.parse::<u32>().ok()?;
    Some(format!("alsa:hw:CARD={card},DEV={dev}"))
}

/// The device to open for `preferred`, falling back to the default when it cannot be found.
pub(crate) fn resolve<D>(
    preferred: Option<&str>,
    find: impl FnOnce(&cpal::DeviceId) -> Option<D>,
    default: impl FnOnce() -> Option<D>,
) -> Option<(D, Pick)> {
    let Some(preferred) = preferred else {
        return default().map(|device| (device, Pick::Default));
    };
    let found = match cpal::DeviceId::from_str(preferred) {
        Ok(id) => find(&id),
        Err(error) => {
            tracing::warn!(device = preferred, %error, "output device id is not readable");
            None
        }
    };
    match found {
        Some(device) => Some((device, Pick::Chosen)),
        None => {
            tracing::warn!(
                device = preferred,
                "chosen output device is not available; using the system default"
            );
            default().map(|device| (device, Pick::Fallback))
        }
    }
}

/// The device the setting names on the default host, with how it was picked; `exclusive` swaps in its raw hardware device.
pub(crate) fn pick_device(
    preferred: Option<&str>,
    exclusive: bool,
) -> Option<(cpal::Device, Pick)> {
    let host = cpal::default_host();
    if exclusive {
        match preferred.and_then(exclusive_target) {
            Some(target) => {
                let found = cpal::DeviceId::from_str(&target)
                    .ok()
                    .and_then(|id| host.device_by_id(&id));
                match found {
                    Some(device) => return Some((device, Pick::Exclusive)),
                    None => tracing::warn!(
                        device = target,
                        "hardware device for exclusive output is not available; sharing the chosen one"
                    ),
                }
            }
            None => tracing::warn!(
                device = ?preferred,
                "exclusive output needs an ALSA hardware device; sharing the chosen one"
            ),
        }
    }
    resolve(
        preferred,
        |id| host.device_by_id(id),
        || host.default_output_device(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_prefix() -> String {
        cpal::ALL_HOSTS[0].to_string()
    }

    #[test]
    fn no_preference_takes_the_default() {
        let picked = resolve(
            None,
            |_| -> Option<&str> { panic!("not asked") },
            || Some("default"),
        );
        assert_eq!(picked, Some(("default", Pick::Default)));
    }

    #[test]
    fn a_present_device_is_opened() {
        let id = format!("{}:hw:CARD=0,DEV=0", host_prefix());
        let picked = resolve(
            Some(&id),
            |asked| (asked.id() == "hw:CARD=0,DEV=0").then_some("dac"),
            || Some("default"),
        );
        assert_eq!(picked, Some(("dac", Pick::Chosen)));
    }

    #[test]
    fn a_missing_device_falls_back_to_the_default() {
        let id = format!("{}:hw:CARD=9,DEV=0", host_prefix());
        let picked = resolve(Some(&id), |_| None, || Some("default"));
        assert_eq!(picked, Some(("default", Pick::Fallback)));
    }

    #[test]
    fn an_unreadable_id_falls_back_to_the_default() {
        for id in ["no-host-separator", "nosuchhost:device"] {
            let picked = resolve(
                Some(id),
                |_| -> Option<&str> { panic!("an unreadable id is never looked up") },
                || Some("default"),
            );
            assert_eq!(picked, Some(("default", Pick::Fallback)), "{id}");
        }
    }

    #[test]
    fn nothing_to_fall_back_to_is_nothing() {
        let picked = resolve(Some("alsa:gone"), |_| None::<&str>, || None);
        assert_eq!(picked, None);
    }

    #[test]
    fn shared_names_are_told_apart_and_the_default_is_marked() {
        let listed = |id: &str, name: &str| Listed {
            id: format!("alsa:{id}"),
            backend_id: id.to_string(),
            name: name.to_string(),
        };
        let devices = labelled(
            vec![
                listed("default", "Default ALSA Output"),
                listed("hw:CARD=PCH,DEV=0", "HDA Intel PCH, ALC892 Analog"),
                listed("plughw:CARD=PCH,DEV=0", "HDA Intel PCH, ALC892 Analog"),
            ],
            Some("alsa:default"),
        );
        let names: Vec<&str> = devices.iter().map(|device| device.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Default ALSA Output",
                "HDA Intel PCH, ALC892 Analog (hw:CARD=PCH,DEV=0)",
                "HDA Intel PCH, ALC892 Analog (plughw:CARD=PCH,DEV=0)",
            ]
        );
        let defaults: Vec<bool> = devices.iter().map(|device| device.is_default).collect();
        assert_eq!(defaults, vec![true, false, false]);
        let exclusive: Vec<bool> = devices.iter().map(|device| device.exclusive).collect();
        assert_eq!(exclusive, vec![false, true, true]);
    }

    #[test]
    fn the_silent_null_device_is_not_offered() {
        let devices = labelled(
            vec![
                Listed {
                    id: "alsa:null".into(),
                    backend_id: "null".into(),
                    name: "Discard all samples (playback) or generate zero samples (capture)"
                        .into(),
                },
                Listed {
                    id: "alsa:pipewire".into(),
                    backend_id: "pipewire".into(),
                    name: "PipeWire Sound Server".into(),
                },
            ],
            Some("alsa:pipewire"),
        );
        let ids: Vec<&str> = devices.iter().map(|device| device.id.as_str()).collect();
        assert_eq!(ids, ["alsa:pipewire"]);
    }

    #[test]
    fn card_level_alsa_ids_map_to_their_hardware_device() {
        for (id, target) in [
            ("alsa:hw:CARD=0,DEV=0", Some("alsa:hw:CARD=0,DEV=0")),
            ("alsa:plughw:CARD=PCH,DEV=3", Some("alsa:hw:CARD=PCH,DEV=3")),
            ("alsa:front:CARD=DAC,DEV=0", Some("alsa:hw:CARD=DAC,DEV=0")),
            ("alsa:sysdefault:CARD=DAC", Some("alsa:hw:CARD=DAC,DEV=0")),
            ("alsa:dmix:CARD=DAC,DEV=1", Some("alsa:hw:CARD=DAC,DEV=1")),
            ("alsa:hw:1,2", Some("alsa:hw:CARD=1,DEV=2")),
            ("alsa:hw:1", Some("alsa:hw:CARD=1,DEV=0")),
            ("alsa:default", None),
            ("alsa:pipewire", None),
            ("alsa:hdmi:CARD=PCH,DEV=0", None),
            ("alsa:hw:CARD=0,DEV=x", None),
            ("alsa:hw:CARD=0,DEV=0,SUBDEV=1", None),
            ("wasapi:{0.0.0.00000000}.{guid}", None),
            ("coreaudio:BuiltInSpeakerDevice", None),
        ] {
            assert_eq!(exclusive_target(id).as_deref(), target, "{id}");
        }
    }

    #[test]
    fn a_chosen_or_exclusive_device_ignores_the_default() {
        assert!(Pick::Default.follows_default());
        assert!(Pick::Fallback.follows_default());
        assert!(!Pick::Chosen.follows_default());
        assert!(!Pick::Exclusive.follows_default());
    }
}
