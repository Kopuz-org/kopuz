use crate::*;

pub fn frontend_pref_to_proto(value: &api::FrontendPref) -> FrontendPref {
    FrontendPref {
        key: value.key.clone(),
        value: value.value.clone(),
    }
}

pub fn frontend_pref_from_proto(value: &FrontendPref) -> api::FrontendPref {
    api::FrontendPref {
        key: value.key.clone(),
        value: value.value.clone(),
    }
}

pub fn pref_entry_to_proto(value: &api::PrefEntry) -> PrefEntry {
    PrefEntry {
        key: value.key.clone(),
        value: value.value.clone(),
    }
}

pub fn pref_entry_from_proto(value: &PrefEntry) -> api::PrefEntry {
    api::PrefEntry {
        key: value.key.clone(),
        value: value.value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_pref_survives_the_wire() {
        for value in ["", "dark", "{\"a\": [1, 2]}\n"] {
            let pref = api::FrontendPref {
                key: "skin".into(),
                value: value.into(),
            };
            assert_eq!(
                frontend_pref_from_proto(&frontend_pref_to_proto(&pref)),
                pref
            );
        }
    }

    /// An empty value stores the empty string; only an absent one deletes.
    #[test]
    fn an_entry_keeps_empty_apart_from_delete() {
        for value in [None, Some(String::new()), Some("x".to_string())] {
            let entry = api::PrefEntry {
                key: "k".into(),
                value,
            };
            assert_eq!(pref_entry_from_proto(&pref_entry_to_proto(&entry)), entry);
        }
        assert_ne!(
            pref_entry_to_proto(&api::PrefEntry {
                key: "k".into(),
                value: None
            }),
            pref_entry_to_proto(&api::PrefEntry {
                key: "k".into(),
                value: Some(String::new())
            }),
        );
    }
}
