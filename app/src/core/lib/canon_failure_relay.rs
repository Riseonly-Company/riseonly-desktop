use std::rc::Rc;

use gpui::{App, Global};

use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};

pub type RiseServerRefusalListener = Rc<dyn Fn(&RiseFailure, &mut App)>;

/// server refusals in canon form as a stream
#[derive(Default)]
pub struct RiseServerRefusalRelay {
    listeners: Vec<RiseServerRefusalListener>,
}

impl Global for RiseServerRefusalRelay {}

impl RiseServerRefusalRelay {
    pub fn subscribe(listener: impl Fn(&RiseFailure, &mut App) + 'static, cx: &mut App) {
        cx.default_global::<Self>().listeners.push(Rc::new(listener));
    }

    /// coded refusals only, a dropped connection or missing code is no classification
    /// restriction is the exception, its restricted_scope detail classifies under any code
    pub fn publish(failure: RiseFailure, cx: &mut App) {
        if CanonErrors::entry(failure.code.as_deref()).is_none() && !failure.is_restriction() {
            return;
        }
        let listeners = match cx.try_global::<Self>() {
            Some(relay) => relay.listeners.clone(),
            None => return,
        };
        for listener in listeners {
            listener(&failure, cx);
        }
    }
}
