use crate::errors::{DialError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedCommand {
    pub value: String,
    pub warning: Option<String>,
}

pub fn sanitize_shell_command(field: &str, value: &str) -> Result<SanitizedCommand> {
    if value.is_empty() || !value.chars().any(is_unicode_dash) {
        return Ok(SanitizedCommand {
            value: value.to_string(),
            warning: None,
        });
    }

    let mut sanitized = String::with_capacity(value.len());
    let mut changed = false;
    let mut token = String::new();

    for ch in value.chars() {
        if ch.is_whitespace() {
            if !token.is_empty() {
                let (new_token, token_changed) = sanitize_token(field, &token)?;
                sanitized.push_str(&new_token);
                changed |= token_changed;
                token.clear();
            }
            sanitized.push(ch);
        } else {
            token.push(ch);
        }
    }

    if !token.is_empty() {
        let (new_token, token_changed) = sanitize_token(field, &token)?;
        sanitized.push_str(&new_token);
        changed |= token_changed;
    }

    let warning = changed.then(|| {
        format!(
            "Normalized Unicode dash characters in {}: '{}' -> '{}'",
            field, value, sanitized
        )
    });

    Ok(SanitizedCommand {
        value: sanitized,
        warning,
    })
}

fn sanitize_token(field: &str, token: &str) -> Result<(String, bool)> {
    if !token.chars().any(is_unicode_dash) {
        return Ok((token.to_string(), false));
    }

    let chars: Vec<char> = token.chars().collect();
    let mut sanitized = String::with_capacity(token.len());
    let mut changed = false;
    let mut allow_interior_dash_fix = token.starts_with('-');

    let mut index = 0;
    let mut prefix_count = 0;
    while index < chars.len() && is_unicode_dash(chars[index]) {
        prefix_count += 1;
        index += 1;
    }

    if prefix_count > 0 {
        let Some(first_rest) = chars.get(index).copied() else {
            return Err(unicode_dash_error(field, token));
        };

        if !is_flag_lead(first_rest) {
            return Err(unicode_dash_error(field, token));
        }

        let body_len = chars[index..]
            .iter()
            .take_while(|c| is_flag_body(**c))
            .count();

        let ascii_prefix = if prefix_count >= 2 || body_len > 1 {
            "--"
        } else {
            "-"
        };
        sanitized.push_str(ascii_prefix);
        changed = true;
        allow_interior_dash_fix = true;
    }

    for pos in index..chars.len() {
        let ch = chars[pos];
        if is_unicode_dash(ch) {
            let prev = sanitized.chars().last();
            let next = chars.get(pos + 1).copied();
            if allow_interior_dash_fix && is_interior_flag_dash(prev, next) {
                sanitized.push('-');
                changed = true;
            } else {
                return Err(unicode_dash_error(field, token));
            }
        } else {
            sanitized.push(ch);
        }
    }

    Ok((sanitized, changed))
}

fn unicode_dash_error(field: &str, token: &str) -> DialError {
    DialError::UserError(format!(
        "{} contains an unsupported Unicode dash in '{}'. Replace it with ASCII '-' or '--'.",
        field, token
    ))
}

fn is_unicode_dash(ch: char) -> bool {
    matches!(
        ch,
        '\u{2010}'
            | '\u{2011}'
            | '\u{2012}'
            | '\u{2013}'
            | '\u{2014}'
            | '\u{2015}'
            | '\u{2212}'
            | '\u{FE58}'
            | '\u{FE63}'
            | '\u{FF0D}'
    )
}

fn is_flag_lead(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
}

fn is_flag_body(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '=' | '.' | '/')
}

fn is_interior_flag_dash(prev: Option<char>, next: Option<char>) -> bool {
    matches!(prev, Some(c) if is_flag_body(c)) && matches!(next, Some(c) if is_flag_body(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_unicode_long_option_prefix() {
        let sanitized = sanitize_shell_command("build command", "cargo —version").unwrap();
        assert_eq!(sanitized.value, "cargo --version");
        assert!(sanitized.warning.is_some());
    }

    #[test]
    fn sanitizes_unicode_short_option_prefix() {
        let sanitized = sanitize_shell_command("build command", "cargo test –q").unwrap();
        assert_eq!(sanitized.value, "cargo test -q");
    }

    #[test]
    fn sanitizes_unicode_dash_inside_option_name() {
        let sanitized =
            sanitize_shell_command("test command", "cargo test --no—default—features").unwrap();
        assert_eq!(sanitized.value, "cargo test --no-default-features");
    }

    #[test]
    fn rejects_ambiguous_unicode_dash_usage() {
        let err = sanitize_shell_command("build command", "echo release—candidate").unwrap_err();
        assert!(
            err.to_string()
                .contains("Replace it with ASCII '-' or '--'"),
            "unexpected error: {err}"
        );
    }
}
