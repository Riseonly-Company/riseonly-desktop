use super::contacts::*;

pub(super) struct NativeContactStore;

impl ContactStore for NativeContactStore {
    fn authorization_status(&self) -> ContactAuthorization {
        ContactAuthorization::Restricted
    }

    fn request_access(&self, done: AccessAnswer) -> Result<(), ContactsError> {
        done(false, None);
        Ok(())
    }
}
