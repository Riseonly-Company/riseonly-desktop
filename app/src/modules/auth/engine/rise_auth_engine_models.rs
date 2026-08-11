use serde::{Deserialize, Serialize};

use super::core::rise_auth_rpc::AuthUserDto;

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct StoredTokens {
    pub access: String,
    pub refresh: String,
    pub session: String,
}

impl StoredTokens {
    pub fn is_complete(&self) -> bool {
        !self.access.trim().is_empty()
            && !self.refresh.trim().is_empty()
            && !self.session.trim().is_empty()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct AuthUser {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub avatar_url: Option<String>,
    pub cover_image_url: Option<String>,
    pub description: Option<String>,
    pub is_premium: bool,
    pub is_official: bool,
    pub role: Option<String>,
    pub user_chat_id: Option<String>,
}

impl From<AuthUserDto> for AuthUser {
    fn from(dto: AuthUserDto) -> Self {
        let more = dto.more.unwrap_or_default();
        Self {
            id: dto.id,
            name: dto.name.unwrap_or_default(),
            tag: dto.tag.unwrap_or_default(),
            avatar_url: dto.avatar_url.or(more.logo),
            cover_image_url: dto.cover_image_url,
            description: more.description,
            is_premium: dto.is_premium.unwrap_or(false),
            is_official: dto.is_official.unwrap_or(false),
            role: dto.role,
            user_chat_id: dto.user_chat_id,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PersistedAccount {
    pub user: AuthUser,
    pub last_used_ms: i64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AccountSummary {
    pub user_id: String,
    pub name: String,
    pub tag: String,
    pub avatar_url: Option<String>,
    pub is_premium: bool,
    pub last_used_ms: i64,
}

impl From<&PersistedAccount> for AccountSummary {
    fn from(account: &PersistedAccount) -> Self {
        Self {
            user_id: account.user.id.clone(),
            name: account.user.name.clone(),
            tag: account.user.tag.clone(),
            avatar_url: account.user.avatar_url.clone(),
            is_premium: account.user.is_premium,
            last_used_ms: account.last_used_ms,
        }
    }
}

pub struct AccountLimitPolicy;

impl AccountLimitPolicy {
    pub const STANDARD: usize = 3;
    pub const PREMIUM: usize = 10;

    pub fn limit(is_premium: bool) -> usize {
        if is_premium {
            Self::PREMIUM
        } else {
            Self::STANDARD
        }
    }

    pub fn can_add(existing: usize, is_premium: bool) -> bool {
        existing < Self::limit(is_premium)
    }
}

pub fn ordered_accounts(
    accounts: &[PersistedAccount],
    active: Option<&str>,
) -> Vec<AccountSummary> {
    let mut current: Vec<AccountSummary> = Vec::new();
    let mut others: Vec<AccountSummary> = Vec::new();

    for account in accounts {
        if Some(account.user.id.as_str()) == active {
            current.push(account.into());
        } else {
            others.push(account.into());
        }
    }

    others.sort_by(|left, right| {
        right
            .last_used_ms
            .cmp(&left.last_used_ms)
            .then_with(|| left.user_id.cmp(&right.user_id))
    });

    current.extend(others);
    current
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResetReason {
    UserLogout,
    ForcedLogout,
    AccountChanged,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::auth::engine::core::rise_auth_rpc::AuthUserMoreDto;

    fn account(id: &str, last_used_ms: i64) -> PersistedAccount {
        PersistedAccount {
            user: AuthUser {
                id: id.to_owned(),
                name: id.to_owned(),
                ..AuthUser::default()
            },
            last_used_ms,
        }
    }

    #[test]
    fn a_user_takes_its_avatar_from_more_logo_when_the_profile_has_none() {
        let user: AuthUser = AuthUserDto {
            id: "u1".into(),
            avatar_url: None,
            more: Some(AuthUserMoreDto {
                logo: Some("https://cdn/logo.png".into()),
                ..AuthUserMoreDto::default()
            }),
            ..AuthUserDto::default()
        }
        .into();

        assert_eq!(user.avatar_url.as_deref(), Some("https://cdn/logo.png"));
    }

    #[test]
    fn an_explicit_avatar_wins_over_the_fallback() {
        let user: AuthUser = AuthUserDto {
            id: "u1".into(),
            avatar_url: Some("https://cdn/avatar.png".into()),
            more: Some(AuthUserMoreDto {
                logo: Some("https://cdn/logo.png".into()),
                ..AuthUserMoreDto::default()
            }),
            ..AuthUserDto::default()
        }
        .into();

        assert_eq!(user.avatar_url.as_deref(), Some("https://cdn/avatar.png"));
    }

    #[test]
    fn the_active_account_is_first_and_the_rest_are_most_recent_first() {
        let accounts = vec![account("a", 10), account("b", 30), account("c", 20)];
        let ordered = ordered_accounts(&accounts, Some("c"));

        assert_eq!(
            ordered
                .iter()
                .map(|summary| summary.user_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "b", "a"]
        );
    }

    #[test]
    fn accounts_used_in_the_same_millisecond_keep_a_stable_order() {
        let accounts = vec![account("b", 5), account("a", 5)];
        let first = ordered_accounts(&accounts, None);
        let second = ordered_accounts(&accounts, None);

        assert_eq!(first, second);
        assert_eq!(first[0].user_id, "a");
    }

    #[test]
    fn an_unknown_active_id_does_not_lose_an_account() {
        let accounts = vec![account("a", 1), account("b", 2)];
        assert_eq!(ordered_accounts(&accounts, Some("missing")).len(), 2);
    }

    #[test]
    fn the_account_limit_is_the_references_own() {
        assert!(AccountLimitPolicy::can_add(2, false));
        assert!(!AccountLimitPolicy::can_add(3, false));
        assert!(AccountLimitPolicy::can_add(3, true));
        assert!(!AccountLimitPolicy::can_add(10, true));
    }

    #[test]
    fn half_a_token_set_is_not_a_session() {
        let complete = StoredTokens {
            access: "a".into(),
            refresh: "r".into(),
            session: "s".into(),
        };
        assert!(complete.is_complete());

        assert!(
            !StoredTokens {
                session: "  ".into(),
                ..complete
            }
            .is_complete()
        );
    }
}
