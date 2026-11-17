use crate::core::config::app_environment::AppEnvironment;
use crate::modules::user::stores::subscription::subscription_services::subscription_services_hooks::SubscriptionServicesHooks;

pub(crate) fn install_subscription_services_hooks() {
    SubscriptionServicesHooks::install(SubscriptionServicesHooks {
        public_site_url: || AppEnvironment::current().public_site_url(),
    });
}
