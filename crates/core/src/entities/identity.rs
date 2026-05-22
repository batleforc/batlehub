use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Anonymous,
    User,
    Admin,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Anonymous => write!(f, "anonymous"),
            Role::User => write!(f, "user"),
            Role::Admin => write!(f, "admin"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub user_id: Option<String>,
    pub role: Role,
    pub auth_provider: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
}

impl Identity {
    pub fn anonymous() -> Self {
        Self {
            user_id: None,
            role: Role::Anonymous,
            auth_provider: None,
            groups: vec![],
        }
    }

    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    pub fn has_role_at_least(&self, minimum: &Role) -> bool {
        &self.role >= minimum
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_admin_true_only_for_admin_role() {
        assert!(Identity { user_id: None, role: Role::Admin, auth_provider: None, groups: vec![] }.is_admin());
        assert!(!Identity { user_id: None, role: Role::User, auth_provider: None, groups: vec![] }.is_admin());
        assert!(!Identity::anonymous().is_admin());
    }

    #[test]
    fn has_role_at_least_respects_ordering() {
        let admin = Identity { user_id: None, role: Role::Admin, auth_provider: None, groups: vec![] };
        let user  = Identity { user_id: None, role: Role::User,  auth_provider: None, groups: vec![] };
        let anon  = Identity::anonymous();

        assert!(admin.has_role_at_least(&Role::Admin));
        assert!(admin.has_role_at_least(&Role::User));
        assert!(admin.has_role_at_least(&Role::Anonymous));

        assert!(!user.has_role_at_least(&Role::Admin));
        assert!(user.has_role_at_least(&Role::User));
        assert!(user.has_role_at_least(&Role::Anonymous));

        assert!(!anon.has_role_at_least(&Role::Admin));
        assert!(!anon.has_role_at_least(&Role::User));
        assert!(anon.has_role_at_least(&Role::Anonymous));
    }

    #[test]
    fn anonymous_returns_anonymous_role_and_no_user_id() {
        let id = Identity::anonymous();
        assert_eq!(id.role, Role::Anonymous);
        assert!(id.user_id.is_none());
        assert!(id.auth_provider.is_none());
    }
}
