use rise_widgets::overlay::premium_gift_celebration_hooks::{
    PremiumGiftCelebrationHooks, set_premium_gift_celebration_hooks,
};

use crate::modules::gifts::shared::gift_people::GiftPeople;

pub(crate) fn install_premium_gift_celebration_hooks() {
    set_premium_gift_celebration_hooks(PremiumGiftCelebrationHooks {
        person_name: |user_id, cx| GiftPeople::person(user_id, cx).map(|person| person.name),
    });
}
