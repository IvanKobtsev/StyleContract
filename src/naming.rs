use clap::ValueEnum;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Convention {
    None,
    CamelCase,
    KebabCase,
    CamelToKebab,
}

impl Convention {
    pub fn valid_code_name(self, value: &str) -> bool {
        match self {
            Self::None => true,
            Self::KebabCase => is_kebab(value),
            Self::CamelCase => is_camel(value),
            Self::CamelToKebab => is_lower_camel(value),
        }
    }

    pub fn valid_style_name(self, value: &str) -> bool {
        match self {
            Self::None => true,
            Self::CamelCase => is_camel(value),
            Self::KebabCase | Self::CamelToKebab => is_kebab(value),
        }
    }

    pub fn code_to_style(self, value: &str) -> String {
        match self {
            Self::None => value.to_owned(),
            Self::CamelToKebab => camel_to_kebab(value),
            _ => value.to_owned(),
        }
    }

    pub fn style_candidates(self, value: &str) -> Vec<String> {
        let exact = value.to_owned();
        if self != Self::None || !is_camel(value) {
            return vec![self.code_to_style(value)];
        }
        let kebab = camel_to_kebab(value);
        if kebab == exact {
            vec![exact]
        } else {
            vec![exact, kebab]
        }
    }
}

pub fn is_camel(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric())
}

fn is_lower_camel(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_lowercase())
        && chars.all(|ch| ch.is_ascii_alphanumeric())
}

pub fn is_kebab(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
        && value
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase())
}

pub fn camel_to_kebab(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if index > 0 {
                output.push('-');
            }
            output.push(ch.to_ascii_lowercase());
        } else {
            output.push(ch);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names() {
        assert!(is_camel("primaryButton2"));
        assert!(is_camel("PrimaryButton"));
        assert!(!is_camel("2PrimaryButton"));
        assert!(!is_camel("primary-button"));
        assert!(is_kebab("primary-button2"));
        assert!(!is_kebab("primary--button"));
        assert!(!is_kebab("primaryButton"));
    }

    #[test]
    fn only_camel_case_accepts_capitalized_component_roots() {
        assert!(Convention::CamelCase.valid_code_name("ComponentRoot"));
        assert!(Convention::CamelCase.valid_style_name("ComponentRoot"));
        assert!(!Convention::CamelToKebab.valid_code_name("ComponentRoot"));
        assert!(!Convention::CamelToKebab.valid_style_name("ComponentRoot"));
        assert!(Convention::CamelToKebab.valid_code_name("componentRoot"));
        assert_eq!(
            Convention::CamelToKebab.code_to_style("componentRoot"),
            "component-root"
        );
    }

    #[test]
    fn conversion_is_deterministic_for_acronyms() {
        assert_eq!(camel_to_kebab("httpURL2Value"), "http-u-r-l2-value");
    }

    #[test]
    fn none_convention_expands_only_camel_style_references() {
        assert_eq!(
            Convention::None.style_candidates("primaryButton"),
            vec!["primaryButton", "primary-button"]
        );
        assert_eq!(
            Convention::None.style_candidates("primary-button"),
            vec!["primary-button"]
        );
        assert_eq!(
            Convention::None.style_candidates("snake_case"),
            vec!["snake_case"]
        );
        assert!(Convention::None.valid_code_name("anything_goes"));
        assert!(Convention::None.valid_style_name("anything_goes"));
    }
}
