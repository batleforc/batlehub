use batlehub_core::entities::Role;

use super::rules::CompiledRule;

pub(super) fn evaluate_auth_rules(
    rules: &[CompiledRule],
    claims: &serde_json::Map<String, serde_json::Value>,
    provider_name: &str,
) -> (Role, Vec<String>) {
    let mut matched_role = Role::Anonymous;
    let mut groups: Vec<String> = Vec::new();
    for rule in rules {
        if rule.evaluate(claims) {
            if let Some(ref role) = rule.role {
                if *role > matched_role {
                    matched_role = role.clone();
                }
            }
            groups.extend(rule.collect_groups(provider_name, claims));
        }
    }
    (matched_role, groups)
}
