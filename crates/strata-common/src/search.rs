//! Search types shared across services (SEARCH-01 … SEARCH-05).
//!
//! Two things live here because both the server and the workflow layer must
//! agree on them:
//!
//! - [`DocumentRef`], the stable reference every document is addressable by
//!   (SEARCH-04). It wraps the [`DocumentId`] in a `strata:doc:<uuid>` string
//!   so links stay valid across storage moves, renames, and refilings.
//! - [`FilterExpr`], the boolean filter language for search strings
//!   (SEARCH-02): `field:value` terms freely combined with `AND`, `OR`,
//!   `NOT`, and parentheses. The expression is plain serializable data, so
//!   workflow steps can build filters as JSON instead of concatenating
//!   strings.

use std::collections::VecDeque;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::DocumentId;

/// Scheme prefix of every stable document reference.
const REF_PREFIX: &str = "strata:doc:";

/// A stable, location-independent reference to a document (SEARCH-04).
///
/// Serialized as `strata:doc:<uuid>`. Unlike a storage path or a folder
/// position, the reference never changes over a document's lifetime, so it
/// is safe to embed in other documents, external systems, and chats. The
/// server resolves references via `GET /refs/{reference}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocumentRef(pub DocumentId);

impl DocumentRef {
    pub fn new(id: DocumentId) -> Self {
        Self(id)
    }
}

impl std::fmt::Display for DocumentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{REF_PREFIX}{}", self.0)
    }
}

/// Error produced when a string is not a valid `strata:doc:<uuid>` reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefParseError(pub String);

impl std::fmt::Display for RefParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not a valid document reference: {}", self.0)
    }
}

impl std::error::Error for RefParseError {}

impl FromStr for DocumentRef {
    type Err = RefParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid = s
            .strip_prefix(REF_PREFIX)
            .ok_or_else(|| RefParseError(s.to_string()))?;
        let uuid = uuid::Uuid::parse_str(uuid).map_err(|_| RefParseError(s.to_string()))?;
        Ok(Self(DocumentId(uuid)))
    }
}

impl Serialize for DocumentRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DocumentRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A boolean filter over document fields (SEARCH-02).
///
/// Built either programmatically (it is plain serializable data, like
/// workflow definitions) or parsed from a search string with
/// [`FilterExpr::parse`]. Evaluation is delegated to a field-lookup closure
/// so this crate stays free of any concrete document representation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterExpr {
    /// All sub-expressions must match.
    And(Vec<FilterExpr>),
    /// At least one sub-expression must match.
    Or(Vec<FilterExpr>),
    /// The sub-expression must not match.
    Not(Box<FilterExpr>),
    /// A single `field:value` comparison. Matches when any of the field's
    /// values equals `value` case-insensitively.
    Term { field: String, value: String },
}

/// Error produced by [`FilterExpr::parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterParseError(pub String);

impl std::fmt::Display for FilterParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid filter: {}", self.0)
    }
}

impl std::error::Error for FilterParseError {}

/// A field referenced in a filter that the evaluator does not know.
///
/// Surfaced as an error instead of "no match" so typos in search strings
/// fail loudly rather than silently returning nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownField(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    LParen,
    RParen,
    And,
    Or,
    Not,
    Term { field: String, value: String },
}

impl FilterExpr {
    /// Parse a filter search string.
    ///
    /// Grammar, loosest binding first: `OR`, then `AND` (also implied
    /// between adjacent terms), then `NOT`, then parentheses. Terms are
    /// `field:value`; values with spaces are quoted (`title:"annual
    /// report"`). Keywords are case-insensitive.
    ///
    /// ```
    /// use strata_common::FilterExpr;
    ///
    /// let expr = FilterExpr::parse(
    ///     r#"type:invoice AND (team:finance OR keyword:urgent) NOT status:draft"#,
    /// ).unwrap();
    /// assert!(matches!(expr, FilterExpr::And(_)));
    /// ```
    pub fn parse(input: &str) -> Result<Self, FilterParseError> {
        let mut tokens = tokenize(input)?;
        let expr = parse_or(&mut tokens)?;
        if let Some(token) = tokens.front() {
            return Err(FilterParseError(format!(
                "unexpected {} after end of expression",
                describe(token)
            )));
        }
        Ok(expr)
    }

    /// Evaluate against a document via `lookup`, which maps a field name to
    /// that document's values for it (multi-valued fields like keywords
    /// return several) or `None` when the field does not exist at all.
    pub fn matches<F>(&self, lookup: &F) -> Result<bool, UnknownField>
    where
        F: Fn(&str) -> Option<Vec<String>>,
    {
        match self {
            FilterExpr::And(parts) => {
                for part in parts {
                    if !part.matches(lookup)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            FilterExpr::Or(parts) => {
                for part in parts {
                    if part.matches(lookup)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            FilterExpr::Not(inner) => Ok(!inner.matches(lookup)?),
            FilterExpr::Term { field, value } => {
                let values = lookup(field).ok_or_else(|| UnknownField(field.clone()))?;
                let value = value.to_lowercase();
                Ok(values.iter().any(|v| v.to_lowercase() == value))
            }
        }
    }
}

fn describe(token: &Token) -> String {
    match token {
        Token::LParen => "'('".into(),
        Token::RParen => "')'".into(),
        Token::And => "'AND'".into(),
        Token::Or => "'OR'".into(),
        Token::Not => "'NOT'".into(),
        Token::Term { field, value } => format!("term '{field}:{value}'"),
    }
}

fn tokenize(input: &str) -> Result<VecDeque<Token>, FilterParseError> {
    let mut tokens = VecDeque::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '(' {
            chars.next();
            tokens.push_back(Token::LParen);
        } else if c == ')' {
            chars.next();
            tokens.push_back(Token::RParen);
        } else {
            let mut word = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '(' || c == ')' {
                    break;
                }
                chars.next();
                word.push(c);
                // A ':' ends the field part; the value may be quoted and
                // contain any character up to the closing quote.
                if c == ':' {
                    break;
                }
            }

            if let Some(field) = word.strip_suffix(':') {
                if field.is_empty() {
                    return Err(FilterParseError("term is missing its field name".into()));
                }
                let value = if chars.peek() == Some(&'"') {
                    chars.next();
                    let mut value = String::new();
                    loop {
                        match chars.next() {
                            Some('"') => break,
                            Some(c) => value.push(c),
                            None => {
                                return Err(FilterParseError(format!(
                                    "unterminated quote in value of '{field}'"
                                )));
                            }
                        }
                    }
                    value
                } else {
                    let mut value = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_whitespace() || c == '(' || c == ')' {
                            break;
                        }
                        chars.next();
                        value.push(c);
                    }
                    value
                };
                if value.is_empty() {
                    return Err(FilterParseError(format!("term '{field}:' has no value")));
                }
                tokens.push_back(Token::Term {
                    field: field.to_string(),
                    value,
                });
            } else {
                match word.to_ascii_uppercase().as_str() {
                    "AND" => tokens.push_back(Token::And),
                    "OR" => tokens.push_back(Token::Or),
                    "NOT" => tokens.push_back(Token::Not),
                    _ => {
                        return Err(FilterParseError(format!(
                            "'{word}' is not a 'field:value' term or operator; \
                             use the text parameter for full-text words"
                        )));
                    }
                }
            }
        }
    }

    Ok(tokens)
}

fn parse_or(tokens: &mut VecDeque<Token>) -> Result<FilterExpr, FilterParseError> {
    let mut parts = vec![parse_and(tokens)?];
    while tokens.front() == Some(&Token::Or) {
        tokens.pop_front();
        parts.push(parse_and(tokens)?);
    }
    Ok(if parts.len() == 1 {
        parts.pop().expect("one part")
    } else {
        FilterExpr::Or(parts)
    })
}

fn parse_and(tokens: &mut VecDeque<Token>) -> Result<FilterExpr, FilterParseError> {
    let mut parts = vec![parse_unary(tokens)?];
    loop {
        match tokens.front() {
            Some(Token::And) => {
                tokens.pop_front();
                parts.push(parse_unary(tokens)?);
            }
            // Adjacent terms imply AND: `type:invoice team:finance`.
            Some(Token::Not) | Some(Token::LParen) | Some(Token::Term { .. }) => {
                parts.push(parse_unary(tokens)?);
            }
            _ => break,
        }
    }
    Ok(if parts.len() == 1 {
        parts.pop().expect("one part")
    } else {
        FilterExpr::And(parts)
    })
}

fn parse_unary(tokens: &mut VecDeque<Token>) -> Result<FilterExpr, FilterParseError> {
    match tokens.pop_front() {
        Some(Token::Not) => Ok(FilterExpr::Not(Box::new(parse_unary(tokens)?))),
        Some(Token::LParen) => {
            let expr = parse_or(tokens)?;
            match tokens.pop_front() {
                Some(Token::RParen) => Ok(expr),
                Some(token) => Err(FilterParseError(format!(
                    "expected ')' but found {}",
                    describe(&token)
                ))),
                None => Err(FilterParseError("missing closing ')'".into())),
            }
        }
        Some(Token::Term { field, value }) => Ok(FilterExpr::Term { field, value }),
        Some(token) => Err(FilterParseError(format!(
            "expected a term, 'NOT', or '(' but found {}",
            describe(&token)
        ))),
        None => Err(FilterParseError("filter is empty".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(field: &str, value: &str) -> FilterExpr {
        FilterExpr::Term {
            field: field.into(),
            value: value.into(),
        }
    }

    #[test]
    fn reference_round_trips_through_string_form() {
        let reference = DocumentRef::new(DocumentId::new());
        let parsed: DocumentRef = reference.to_string().parse().unwrap();
        assert_eq!(parsed, reference);
    }

    #[test]
    fn reference_serializes_as_prefixed_string() {
        let id = DocumentId::new();
        let json = serde_json::to_string(&DocumentRef::new(id)).unwrap();
        assert_eq!(json, format!("\"strata:doc:{id}\""));
    }

    #[test]
    fn reference_rejects_other_schemes_and_bad_uuids() {
        assert!("https://example.com/doc/1".parse::<DocumentRef>().is_err());
        assert!("strata:doc:not-a-uuid".parse::<DocumentRef>().is_err());
        assert!(
            "strata:dossier:5bf9cbb8-6a56-44e5-a35f-30d885bd5c31"
                .parse::<DocumentRef>()
                .is_err()
        );
    }

    #[test]
    fn parses_single_term() {
        assert_eq!(
            FilterExpr::parse("type:invoice").unwrap(),
            term("type", "invoice")
        );
    }

    #[test]
    fn parses_quoted_values_with_spaces() {
        assert_eq!(
            FilterExpr::parse(r#"title:"annual report""#).unwrap(),
            term("title", "annual report")
        );
    }

    #[test]
    fn and_binds_tighter_than_or() {
        assert_eq!(
            FilterExpr::parse("type:invoice AND team:finance OR team:hr").unwrap(),
            FilterExpr::Or(vec![
                FilterExpr::And(vec![term("type", "invoice"), term("team", "finance")]),
                term("team", "hr"),
            ])
        );
    }

    #[test]
    fn adjacent_terms_imply_and() {
        assert_eq!(
            FilterExpr::parse("type:invoice team:finance").unwrap(),
            FilterExpr::And(vec![term("type", "invoice"), term("team", "finance")])
        );
    }

    #[test]
    fn parentheses_override_precedence() {
        assert_eq!(
            FilterExpr::parse("type:invoice AND (team:finance OR team:hr)").unwrap(),
            FilterExpr::And(vec![
                term("type", "invoice"),
                FilterExpr::Or(vec![term("team", "finance"), term("team", "hr")]),
            ])
        );
    }

    #[test]
    fn not_negates_and_operators_are_case_insensitive() {
        assert_eq!(
            FilterExpr::parse("not status:draft and type:invoice").unwrap(),
            FilterExpr::And(vec![
                FilterExpr::Not(Box::new(term("status", "draft"))),
                term("type", "invoice"),
            ])
        );
    }

    #[test]
    fn rejects_bare_words_unbalanced_parens_and_empty_input() {
        assert!(FilterExpr::parse("invoice").is_err());
        assert!(FilterExpr::parse("(type:invoice").is_err());
        assert!(FilterExpr::parse("type:invoice)").is_err());
        assert!(FilterExpr::parse("").is_err());
        assert!(FilterExpr::parse("type:").is_err());
        assert!(FilterExpr::parse(r#"title:"unterminated"#).is_err());
    }

    #[test]
    fn matches_evaluates_boolean_logic_case_insensitively() {
        let expr =
            FilterExpr::parse("type:Invoice AND (team:finance OR keyword:urgent) NOT status:draft")
                .unwrap();
        let lookup = |field: &str| -> Option<Vec<String>> {
            match field {
                "type" => Some(vec!["invoice".into()]),
                "team" => Some(vec!["legal".into()]),
                "keyword" => Some(vec!["Urgent".into(), "q3".into()]),
                "status" => Some(vec!["in_use".into()]),
                _ => None,
            }
        };
        assert!(expr.matches(&lookup).unwrap());

        let expr = FilterExpr::parse("team:finance").unwrap();
        assert!(!expr.matches(&lookup).unwrap());
    }

    #[test]
    fn matches_reports_unknown_fields() {
        let expr = FilterExpr::parse("nope:x").unwrap();
        let lookup = |_: &str| -> Option<Vec<String>> { None };
        assert_eq!(expr.matches(&lookup), Err(UnknownField("nope".into())));
    }

    #[test]
    fn filter_expr_is_plain_serializable_data() {
        let expr = FilterExpr::parse("type:invoice AND NOT status:draft").unwrap();
        let json = serde_json::to_value(&expr).unwrap();
        let back: FilterExpr = serde_json::from_value(json).unwrap();
        assert_eq!(back, expr);
    }
}
