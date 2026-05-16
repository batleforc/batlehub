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
}

impl Identity {
    pub fn anonymous() -> Self {
        Self {
            user_id: None,
            role: Role::Anonymous,
            auth_provider: None,
        }
    }

    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }

    pub fn has_role_at_least(&self, minimum: &Role) -> bool {
        &self.role >= minimum
    }
}
