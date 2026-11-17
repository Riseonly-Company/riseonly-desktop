use super::ShareTagPolicy;

#[test]
fn tag_sharing_always_uses_exactly_one_at_sign() {
    assert_eq!(ShareTagPolicy::normalized("nics51").as_deref(), Some("@nics51"));
    assert_eq!(ShareTagPolicy::normalized(" @nics51 ").as_deref(), Some("@nics51"));
    assert_eq!(ShareTagPolicy::normalized("@@nics51").as_deref(), Some("@nics51"));
    assert_eq!(ShareTagPolicy::normalized(" @ "), None);
}
