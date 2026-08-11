use rise_ui::phone_input::{PhoneCountryCatalog, PhoneNumberEntry, countries};

use crate::core::animations::Animation;
use crate::modules::auth::shared::auth_validation;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuthStep {
    Phone,
    LoginPassword,
    RegistrationName,
    RegistrationTag,
    RegistrationPassword,
    RegistrationPasswordConfirmation,
    VerificationCode,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TagMood {
    #[default]
    Unknown,
    Available,
    Rejected,
}

impl AuthStep {
    pub fn title_key(self) -> &'static str {
        match self {
            Self::Phone => "auth_phone_step_title",
            Self::LoginPassword => "auth_login_password_step_title",
            Self::RegistrationName => "auth_name_step_title",
            Self::RegistrationTag => "auth_tag_step_title",
            Self::RegistrationPassword => "auth_password_step_title",
            Self::RegistrationPasswordConfirmation => "auth_repeat_password_step_title",
            Self::VerificationCode => "send_code",
        }
    }

    pub fn subtitle_key(self) -> &'static str {
        match self {
            Self::Phone => "auth_phone_step_subtitle",
            Self::LoginPassword => "auth_login_password_step_subtitle",
            Self::RegistrationName => "auth_name_step_subtitle",
            Self::RegistrationTag => "auth_tag_step_subtitle",
            Self::RegistrationPassword => "auth_password_step_subtitle",
            Self::RegistrationPasswordConfirmation => "auth_repeat_password_step_subtitle",
            Self::VerificationCode => "send_code_subtitle",
        }
    }

    pub fn placeholder_key(self) -> &'static str {
        match self {
            Self::Phone => "phone_number_placeholder",
            Self::LoginPassword | Self::RegistrationPassword => "password_placeholder",
            Self::RegistrationName => "name_placeholder",
            Self::RegistrationTag => "auth_tag_placeholder",
            Self::RegistrationPasswordConfirmation => "repeat_password_placeholder",
            Self::VerificationCode => "send_code",
        }
    }

    pub fn animation(self, tag: TagMood) -> &'static str {
        match self {
            Self::Phone => Animation::HELLO,
            Self::LoginPassword => Animation::QUITE,
            Self::RegistrationName => Animation::GREETINGS,
            Self::RegistrationTag => match tag {
                TagMood::Available => Animation::LIKE,
                TagMood::Rejected => Animation::NO,
                TagMood::Unknown => Animation::WHAT,
            },
            Self::RegistrationPassword => Animation::WORK,
            Self::RegistrationPasswordConfirmation => Animation::HELP,
            Self::VerificationCode => Animation::GIFT,
        }
    }

    pub fn action_key(self) -> Option<&'static str> {
        match self {
            Self::LoginPassword => Some("signin"),
            Self::RegistrationPasswordConfirmation => Some("auth_get_code"),
            Self::VerificationCode => Some("signup"),
            _ => None,
        }
    }

    pub fn is_secure(self) -> bool {
        matches!(
            self,
            Self::LoginPassword
                | Self::RegistrationPassword
                | Self::RegistrationPasswordConfirmation
        )
    }

    pub fn progress(self, registering: bool) -> (usize, usize) {
        let total = if registering { 6 } else { 2 };
        let index = match self {
            Self::Phone => 0,
            Self::LoginPassword | Self::RegistrationName => 1,
            Self::RegistrationTag => 2,
            Self::RegistrationPassword => 3,
            Self::RegistrationPasswordConfirmation => 4,
            Self::VerificationCode => 5,
        };
        (index.min(total - 1), total)
    }

    pub fn previous(self) -> Option<Self> {
        match self {
            Self::Phone => None,
            Self::LoginPassword | Self::RegistrationName => Some(Self::Phone),
            Self::RegistrationTag => Some(Self::RegistrationName),
            Self::RegistrationPassword => Some(Self::RegistrationTag),
            Self::RegistrationPasswordConfirmation => Some(Self::RegistrationPassword),
            Self::VerificationCode => Some(Self::RegistrationPasswordConfirmation),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuthStepForm {
    pub phone: PhoneNumberEntry,
    pub login_password: String,
    pub name: String,
    pub tag: String,
    pub registration_password: String,
    pub repeated_password: String,
    pub code: String,
}

impl AuthStepForm {
    pub fn detected(catalog: &PhoneCountryCatalog, device_region: Option<&str>) -> Self {
        Self {
            phone: PhoneNumberEntry::detected(catalog, device_region),
            login_password: String::new(),
            name: String::new(),
            tag: String::new(),
            registration_password: String::new(),
            repeated_password: String::new(),
            code: String::new(),
        }
    }

    pub fn composed_phone(&self) -> String {
        self.phone.wire_value()
    }

    pub fn value_for(&self, step: AuthStep) -> &str {
        match step {
            AuthStep::Phone => self.phone.national(),
            AuthStep::LoginPassword => &self.login_password,
            AuthStep::RegistrationName => &self.name,
            AuthStep::RegistrationTag => &self.tag,
            AuthStep::RegistrationPassword => &self.registration_password,
            AuthStep::RegistrationPasswordConfirmation => &self.repeated_password,
            AuthStep::VerificationCode => &self.code,
        }
    }

    pub fn set(&mut self, step: AuthStep, value: String) {
        match step {
            AuthStep::Phone => {
                self.phone.accept_typed(&value, countries());
            }
            AuthStep::LoginPassword => self.login_password = value,
            AuthStep::RegistrationName => self.name = value,
            AuthStep::RegistrationTag => self.tag = value,
            AuthStep::RegistrationPassword => self.registration_password = value,
            AuthStep::RegistrationPasswordConfirmation => self.repeated_password = value,
            AuthStep::VerificationCode => self.code = value,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepValidation {
    Valid,
    Invalid(&'static str),
}

impl StepValidation {
    pub fn is_valid(self) -> bool {
        matches!(self, Self::Valid)
    }

    pub fn message_key(self) -> Option<&'static str> {
        match self {
            Self::Valid => None,
            Self::Invalid(key) => Some(key),
        }
    }
}

pub fn validate(step: AuthStep, form: &AuthStepForm) -> StepValidation {
    let judged = |outcome: Result<String, rise_validate::Violation>| match outcome {
        Ok(_) => StepValidation::Valid,
        Err(violation) => StepValidation::Invalid(auth_validation::message_key(&violation)),
    };

    match step {
        AuthStep::Phone => judged(auth_validation::PHONE.check(&form.composed_phone())),
        AuthStep::LoginPassword => {
            judged(auth_validation::LOGIN_PASSWORD.check(&form.login_password))
        }
        AuthStep::RegistrationName => judged(auth_validation::NAME.check(&form.name)),
        AuthStep::RegistrationTag => judged(auth_validation::TAG.check(&form.tag)),
        AuthStep::RegistrationPassword => {
            judged(auth_validation::PASSWORD.check(&form.registration_password))
        }
        AuthStep::RegistrationPasswordConfirmation => {
            if form.repeated_password.is_empty() {
                return StepValidation::Invalid("auth_password_required");
            }
            if rise_validate::confirm(
                &auth_validation::PASSWORD,
                &form.registration_password,
                &form.repeated_password,
            )
            .is_err()
            {
                return StepValidation::Invalid("auth_password_mismatch");
            }
            StepValidation::Valid
        }
        AuthStep::VerificationCode => judged(auth_validation::CODE.check(&form.code)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> PhoneCountryCatalog {
        PhoneCountryCatalog::parse(
            "7;KZ;XXX XXX XX XX;Kazakhstan\n1;US;XXX XXX XXXX;United States\n",
        )
    }

    fn form() -> AuthStepForm {
        let catalog = catalog();
        let mut form = AuthStepForm {
            phone: PhoneNumberEntry::detected(&catalog, Some("KZ")),
            login_password: "password".into(),
            name: "Name".into(),
            tag: "riseonly".into(),
            registration_password: "password".into(),
            repeated_password: "password".into(),
            code: "1234".into(),
        };
        form.phone.accept_typed("7075803272", &catalog);
        form
    }

    const EVERY_STEP: [AuthStep; 7] = [
        AuthStep::Phone,
        AuthStep::LoginPassword,
        AuthStep::RegistrationName,
        AuthStep::RegistrationTag,
        AuthStep::RegistrationPassword,
        AuthStep::RegistrationPasswordConfirmation,
        AuthStep::VerificationCode,
    ];

    #[test]
    fn a_complete_form_passes_every_step() {
        for step in EVERY_STEP {
            assert!(
                validate(step, &form()).is_valid(),
                "{step:?} refused a form the server would accept"
            );
        }
    }

    #[test]
    fn the_back_walk_from_the_code_step_reaches_the_phone_in_five_moves() {
        let mut step = AuthStep::VerificationCode;
        let mut moves = 0;
        while let Some(previous) = step.previous() {
            step = previous;
            moves += 1;
            assert!(moves < 10, "the back-walk does not terminate");
        }
        assert_eq!(step, AuthStep::Phone);
        assert_eq!(
            moves, 5,
            "registration walks back through what it collected rather than resetting"
        );
    }

    #[test]
    fn signing_in_never_refuses_a_password_the_server_would_accept() {
        let mut short = form();
        short.login_password = "12".into();
        assert!(
            validate(AuthStep::LoginPassword, &short).is_valid(),
            "auth-service does not check the length on the login path"
        );

        short.login_password = String::new();
        assert!(!validate(AuthStep::LoginPassword, &short).is_valid());
    }

    #[test]
    fn registration_does_apply_the_password_bound_the_server_has() {
        let mut short = form();
        short.registration_password = "12345".into();
        assert_eq!(
            validate(AuthStep::RegistrationPassword, &short).message_key(),
            Some("auth_password_too_short")
        );
    }

    #[test]
    fn a_mismatched_confirmation_names_the_mismatch_rather_than_a_length() {
        let mut mismatched = form();
        mismatched.repeated_password = "different".into();
        assert_eq!(
            validate(AuthStep::RegistrationPasswordConfirmation, &mismatched).message_key(),
            Some("auth_password_mismatch")
        );
    }

    #[test]
    fn every_step_names_a_title_a_subtitle_and_a_placeholder() {
        for step in EVERY_STEP {
            assert!(!step.title_key().is_empty());
            assert!(!step.subtitle_key().is_empty());
            assert!(!step.placeholder_key().is_empty());
        }
    }

    #[test]
    fn every_step_names_an_animation_the_product_ships() {
        for step in EVERY_STEP {
            for mood in [TagMood::Unknown, TagMood::Available, TagMood::Rejected] {
                let animation = step.animation(mood);
                assert!(
                    Animation::ALL.contains(&animation),
                    "{step:?}/{mood:?} names {animation}, which is not shipped"
                );
            }
        }
    }

    #[test]
    fn the_tag_step_is_the_only_one_whose_animation_reacts_to_availability() {
        for step in EVERY_STEP {
            let reacts = step.animation(TagMood::Available) != step.animation(TagMood::Rejected);
            assert_eq!(
                reacts,
                step == AuthStep::RegistrationTag,
                "{step:?} changes its animation on a value it does not show"
            );
        }
    }

    #[test]
    fn only_the_password_steps_are_secure() {
        for step in EVERY_STEP {
            assert_eq!(
                step.is_secure(),
                matches!(
                    step,
                    AuthStep::LoginPassword
                        | AuthStep::RegistrationPassword
                        | AuthStep::RegistrationPasswordConfirmation
                ),
                "{step:?} shows or hides its characters wrongly"
            );
        }
    }

    #[test]
    fn signing_in_shows_a_two_segment_bar_and_registering_a_six() {
        assert_eq!(AuthStep::Phone.progress(false), (0, 2));
        assert_eq!(AuthStep::LoginPassword.progress(false), (1, 2));

        assert_eq!(AuthStep::Phone.progress(true), (0, 6));
        assert_eq!(AuthStep::VerificationCode.progress(true), (5, 6));
    }

    #[test]
    fn no_step_ever_indexes_past_the_bar_it_is_drawn_in() {
        for step in EVERY_STEP {
            for registering in [true, false] {
                let (index, total) = step.progress(registering);
                assert!(index < total, "{step:?} registering={registering}");
            }
        }
    }

    #[test]
    fn a_value_written_to_a_step_reads_back_from_it() {
        let mut form = form();
        for step in EVERY_STEP {
            if step == AuthStep::Phone {
                continue;
            }
            form.set(step, format!("{step:?}"));
        }
        for step in EVERY_STEP {
            if step == AuthStep::Phone {
                continue;
            }
            assert_eq!(form.value_for(step), format!("{step:?}"));
        }
    }

    #[test]
    fn a_phone_typed_in_full_reaches_the_wire_without_a_doubled_country_code() {
        let catalog = catalog();
        let mut form = form();

        form.phone.accept_typed("77075803272", &catalog);

        assert_eq!(form.composed_phone(), "77075803272");
        assert!(validate(AuthStep::Phone, &form).is_valid());
    }
}
