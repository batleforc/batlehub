use batlehub_core::entities::Role;
use batlehub_core::ports::{ActionsGroupRule, ConditionMatchType, RuleMatch};

pub(super) enum CompiledCondition {
    Glob {
        claim: String,
        pattern: glob::Pattern,
    },
    Regex {
        claim: String,
        re: regex::Regex,
    },
}

impl CompiledCondition {
    pub(super) fn compile(c: &batlehub_core::ports::Condition) -> anyhow::Result<Self> {
        let is_regex = match c.match_type {
            ConditionMatchType::Regex => true,
            ConditionMatchType::Glob => false,
            ConditionMatchType::Auto => detect_is_regex(&c.pattern),
        };
        if is_regex {
            let re = regex::Regex::new(&c.pattern)
                .map_err(|e| anyhow::anyhow!("invalid regex pattern {:?}: {e}", c.pattern))?;
            Ok(Self::Regex {
                claim: c.claim.clone(),
                re,
            })
        } else {
            let pattern = glob::Pattern::new(&c.pattern)
                .map_err(|e| anyhow::anyhow!("invalid glob pattern {:?}: {e}", c.pattern))?;
            Ok(Self::Glob {
                claim: c.claim.clone(),
                pattern,
            })
        }
    }

    pub(super) fn matches(&self, claims: &serde_json::Map<String, serde_json::Value>) -> bool {
        match self {
            Self::Glob { claim, pattern } => {
                let val = claim_str(claims, claim);
                pattern.matches(val)
            }
            Self::Regex { claim, re } => {
                let val = claim_str(claims, claim);
                re.is_match(val)
            }
        }
    }
}

pub(super) fn detect_is_regex(pattern: &str) -> bool {
    pattern.starts_with('^')
        || pattern.ends_with('$')
        || pattern.contains("(?")
        || pattern.contains("\\d")
        || pattern.contains("\\w")
        || pattern.contains('[')
        || pattern.contains('(')
        || pattern.contains('+')
}

pub(super) fn claim_str<'a>(
    claims: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> &'a str {
    claims.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

pub(super) struct CompiledRule {
    pub(super) static_group: Option<String>,
    pub(super) group_template: Option<String>,
    /// `None` means the rule contributes groups only, without affecting role elevation.
    pub(super) role: Option<Role>,
    pub(super) conditions: Vec<CompiledCondition>,
    pub(super) match_mode: RuleMatch,
}

impl CompiledRule {
    pub(super) fn compile(rule: &ActionsGroupRule) -> anyhow::Result<Self> {
        if rule.group.is_none() && rule.group_template.is_none() {
            anyhow::bail!("each rule must have at least one of 'group' or 'group_template'");
        }
        let role = rule.role.as_deref().map(parse_role);
        let conditions = rule
            .conditions
            .iter()
            .map(CompiledCondition::compile)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            static_group: rule.group.clone(),
            group_template: rule.group_template.clone(),
            role,
            conditions,
            match_mode: rule.match_mode.clone(),
        })
    }

    pub(super) fn evaluate(&self, claims: &serde_json::Map<String, serde_json::Value>) -> bool {
        if self.conditions.is_empty() {
            return true;
        }
        match self.match_mode {
            RuleMatch::All => self.conditions.iter().all(|c| c.matches(claims)),
            RuleMatch::Any => self.conditions.iter().any(|c| c.matches(claims)),
        }
    }

    pub(super) fn collect_groups(
        &self,
        provider_name: &str,
        claims: &serde_json::Map<String, serde_json::Value>,
    ) -> Vec<String> {
        let mut groups = Vec::new();
        if let Some(g) = &self.static_group {
            groups.push(g.clone());
        }
        if let Some(t) = &self.group_template {
            groups.push(render_group_template(t, provider_name, claims));
        }
        groups
    }
}

pub(super) fn parse_role(s: &str) -> Role {
    match s {
        "admin" => Role::Admin,
        "user" => Role::User,
        _ => Role::Anonymous,
    }
}

pub(super) fn extract_ref_name(claims: &serde_json::Map<String, serde_json::Value>) -> String {
    claims
        .get("ref")
        .and_then(|v| v.as_str())
        .map(|r| {
            r.strip_prefix("refs/heads/")
                .or_else(|| r.strip_prefix("refs/tags/"))
                .unwrap_or(r)
                .replace('/', "-")
        })
        .unwrap_or_default()
}

pub(super) fn substitute_placeholder(
    key: &str,
    provider_name: &str,
    ref_name: &str,
    claims: &serde_json::Map<String, serde_json::Value>,
) -> String {
    if key == "name" {
        provider_name.replace('/', "-")
    } else if key == "ref_name" {
        ref_name.to_owned()
    } else {
        claims
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.replace('/', "-"))
            .unwrap_or_else(|| format!("{{{key}}}"))
    }
}

/// Render a group name template by substituting `{placeholders}` with claim values.
///
/// Special variables:
/// - `{name}`: the provider's configured name
/// - `{ref_name}`: the `ref` claim with `refs/heads/` or `refs/tags/` prefix stripped
/// - `{any_claim}`: the value of that JWT claim
///
/// Substituted values have `/` replaced with `-` so group names stay path-safe.
/// Template literal `/` separators are preserved unchanged.
pub(super) fn render_group_template(
    template: &str,
    provider_name: &str,
    claims: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let ref_name = extract_ref_name(claims);
    let mut result = String::with_capacity(template.len() + 16);
    let mut chars = template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            let mut key = String::new();
            let mut closed = false;
            for c in chars.by_ref() {
                if c == '}' {
                    closed = true;
                    break;
                }
                key.push(c);
            }
            if closed {
                result.push_str(&substitute_placeholder(
                    &key,
                    provider_name,
                    &ref_name,
                    claims,
                ));
            } else {
                result.push('{');
                result.push_str(&key);
            }
        } else {
            result.push(ch);
        }
    }
    result
}
