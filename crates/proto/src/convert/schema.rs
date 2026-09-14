use crate::*;

pub fn text_to_proto(value: &api::Text) -> Text {
    let (kind, text) = match value {
        api::Text::Key(key) => (TextKind::Key, key),
        api::Text::Literal(literal) => (TextKind::Literal, literal),
    };
    Text {
        kind: kind as i32,
        value: text.clone(),
    }
}

pub fn text_from_proto(value: &Text) -> api::Text {
    match TextKind::try_from(value.kind).unwrap_or(TextKind::Unspecified) {
        TextKind::Key => api::Text::Key(value.value.clone()),
        TextKind::Literal | TextKind::Unspecified => api::Text::Literal(value.value.clone()),
    }
}

pub fn icon_to_proto(value: &api::Icon) -> Icon {
    let (kind, glyph) = match value {
        api::Icon::Class(class) => (IconKind::Class, class),
        api::Icon::Svg(path) => (IconKind::Svg, path),
    };
    Icon {
        kind: kind as i32,
        value: glyph.clone(),
    }
}

pub fn icon_from_proto(value: &Icon) -> api::Icon {
    match IconKind::try_from(value.kind).unwrap_or(IconKind::Unspecified) {
        IconKind::Svg => api::Icon::Svg(value.value.clone()),
        IconKind::Class | IconKind::Unspecified => api::Icon::Class(value.value.clone()),
    }
}

pub fn choice_option_to_proto(value: &api::ChoiceOption) -> ChoiceOption {
    ChoiceOption {
        value: value.value.clone(),
        label: Some(text_to_proto(&value.label)),
    }
}

pub fn choice_option_from_proto(value: &ChoiceOption) -> api::ChoiceOption {
    api::ChoiceOption {
        value: value.value.clone(),
        label: value
            .label
            .as_ref()
            .map(text_from_proto)
            .unwrap_or_default(),
    }
}

pub fn field_kind_to_proto(value: &api::FieldKind) -> FieldKind {
    let tag = match value {
        api::FieldKind::Text => FieldKindTag::Text,
        api::FieldKind::Secret => FieldKindTag::Secret,
        api::FieldKind::Url => FieldKindTag::Url,
        api::FieldKind::Toggle => FieldKindTag::Toggle,
        api::FieldKind::Directory => FieldKindTag::Directory,
        api::FieldKind::Choice { .. } => FieldKindTag::Choice,
        api::FieldKind::Radio { .. } => FieldKindTag::Radio,
        api::FieldKind::Note => FieldKindTag::Note,
    };
    let options = match value {
        api::FieldKind::Choice { options, .. } | api::FieldKind::Radio { options } => {
            options.iter().map(choice_option_to_proto).collect()
        }
        _ => Vec::new(),
    };
    FieldKind {
        tag: tag as i32,
        options,
        custom: matches!(value, api::FieldKind::Choice { custom: true, .. }),
    }
}

pub fn field_kind_from_proto(value: &FieldKind) -> api::FieldKind {
    let options = || -> Vec<api::ChoiceOption> {
        value
            .options
            .iter()
            .map(choice_option_from_proto)
            .collect::<Vec<_>>()
    };
    match FieldKindTag::try_from(value.tag).unwrap_or(FieldKindTag::Unspecified) {
        FieldKindTag::Secret => api::FieldKind::Secret,
        FieldKindTag::Url => api::FieldKind::Url,
        FieldKindTag::Toggle => api::FieldKind::Toggle,
        FieldKindTag::Directory => api::FieldKind::Directory,
        FieldKindTag::Choice => api::FieldKind::Choice {
            options: options(),
            custom: value.custom,
        },
        FieldKindTag::Radio => api::FieldKind::Radio { options: options() },
        FieldKindTag::Note => api::FieldKind::Note,
        FieldKindTag::Text | FieldKindTag::Unspecified => api::FieldKind::Text,
    }
}

pub fn field_spec_to_proto(value: &api::FieldSpec) -> FieldSpec {
    FieldSpec {
        key: value.key.clone(),
        label: Some(text_to_proto(&value.label)),
        help: value.help.as_ref().map(text_to_proto),
        placeholder: value.placeholder.as_ref().map(text_to_proto),
        section: value.section.as_ref().map(text_to_proto),
        kind: Some(field_kind_to_proto(&value.kind)),
        required: value.required,
        value: value.value.clone(),
        config_key: value.config_key.clone(),
        show_when: value.show_when.as_ref().map(field_value_to_proto),
    }
}

pub fn field_spec_from_proto(value: &FieldSpec) -> api::FieldSpec {
    api::FieldSpec {
        key: value.key.clone(),
        label: value
            .label
            .as_ref()
            .map(text_from_proto)
            .unwrap_or_default(),
        help: value.help.as_ref().map(text_from_proto),
        placeholder: value.placeholder.as_ref().map(text_from_proto),
        section: value.section.as_ref().map(text_from_proto),
        kind: value
            .kind
            .as_ref()
            .map(field_kind_from_proto)
            .unwrap_or_default(),
        required: value.required,
        value: value.value.clone(),
        config_key: value.config_key.clone(),
        show_when: value.show_when.as_ref().map(field_value_from_proto),
    }
}

pub fn field_value_to_proto(value: &api::FieldValue) -> FieldValue {
    FieldValue {
        key: value.key.clone(),
        value: value.value.clone(),
    }
}

pub fn field_value_from_proto(value: &FieldValue) -> api::FieldValue {
    api::FieldValue {
        key: value.key.clone(),
        value: value.value.clone(),
    }
}

pub fn problem_to_proto(value: &api::Problem) -> Problem {
    Problem {
        field: value.field.clone(),
        label: Some(text_to_proto(&value.label)),
    }
}

pub fn problem_from_proto(value: &Problem) -> api::Problem {
    api::Problem {
        field: value.field.clone(),
        label: value
            .label
            .as_ref()
            .map(text_from_proto)
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: api::FieldKind) -> api::FieldSpec {
        api::FieldSpec {
            key: "url".into(),
            label: api::Text::key("settings-url"),
            help: Some(api::Text::literal("https://jelly.example")),
            placeholder: None,
            section: Some(api::Text::key("settings-server")),
            kind,
            required: true,
            value: Some("https://jelly.example".into()),
            config_key: Some("servers.url".into()),
            show_when: Some(api::FieldValue::new("anonymous", "false")),
        }
    }

    /// A field list is the whole settings surface a client renders, so every
    /// control it can name has to survive the wire.
    #[test]
    fn every_field_kind_round_trips() {
        let options = vec![
            api::ChoiceOption {
                value: "us".into(),
                label: api::Text::literal("United States"),
            },
            api::ChoiceOption {
                value: "tr".into(),
                label: api::Text::key("storefront-tr"),
            },
        ];
        let kinds = [
            api::FieldKind::Text,
            api::FieldKind::Secret,
            api::FieldKind::Url,
            api::FieldKind::Toggle,
            api::FieldKind::Directory,
            api::FieldKind::Choice {
                options: options.clone(),
                custom: true,
            },
            api::FieldKind::Choice {
                options: options.clone(),
                custom: false,
            },
            api::FieldKind::Radio { options },
            api::FieldKind::Note,
        ];
        for kind in kinds {
            let field = spec(kind);
            assert_eq!(field, field_spec_from_proto(&field_spec_to_proto(&field)));
        }
    }

    #[test]
    fn both_text_and_icon_variants_round_trip() {
        for text in [api::Text::key("k"), api::Text::literal("Jellyfin")] {
            assert_eq!(text, text_from_proto(&text_to_proto(&text)));
        }
        for icon in [
            api::Icon::Class("ph-cloud".into()),
            api::Icon::Svg("M0 0h24v24H0z".into()),
        ] {
            assert_eq!(icon, icon_from_proto(&icon_to_proto(&icon)));
        }
    }

    /// An unknown tag is the api default, never a panic: a newer daemon may
    /// name a control this build has no renderer for.
    #[test]
    fn unknown_wire_values_fall_back_to_the_default() {
        let kind = FieldKind {
            tag: 404,
            ..Default::default()
        };
        assert_eq!(field_kind_from_proto(&kind), api::FieldKind::Text);
        let text = Text {
            kind: 404,
            value: "x".into(),
        };
        assert_eq!(text_from_proto(&text), api::Text::Literal("x".into()));
    }

    #[test]
    fn a_problem_keeps_the_field_it_belongs_under() {
        let problem = api::Problem::on("url", api::Text::key("error-url-empty"));
        assert_eq!(problem, problem_from_proto(&problem_to_proto(&problem)));
        let general = api::Problem {
            field: None,
            label: api::Text::key("error-unreachable"),
        };
        assert_eq!(general, problem_from_proto(&problem_to_proto(&general)));
    }
}
