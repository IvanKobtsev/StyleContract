use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Convention {
    CamelCase,
    KebabCase,
    CamelToKebab,
}

impl Convention {
    pub fn valid_code_name(self, value: &str) -> bool {
        match self {
            Self::KebabCase => is_kebab(value),
            Self::CamelCase | Self::CamelToKebab => is_camel(value),
        }
    }

    pub fn valid_style_name(self, value: &str) -> bool {
        match self {
            Self::CamelCase => is_camel(value),
            Self::KebabCase | Self::CamelToKebab => is_kebab(value),
        }
    }

    pub fn code_to_style(self, value: &str) -> String {
        match self {
            Self::CamelToKebab => camel_to_kebab(value),
            _ => value.to_owned(),
        }
    }
}

pub fn is_camel(value: &str) -> bool {
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
        assert!(!is_camel("PrimaryButton"));
        assert!(!is_camel("primary-button"));
        assert!(is_kebab("primary-button2"));
        assert!(!is_kebab("primary--button"));
        assert!(!is_kebab("primaryButton"));
    }

    #[test]
    fn conversion_is_deterministic_for_acronyms() {
        assert_eq!(camel_to_kebab("httpURL2Value"), "http-u-r-l2-value");
    }
}
