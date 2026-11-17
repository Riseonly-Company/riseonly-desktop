use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::map_view::LatLng;
use serde::{Deserialize, Serialize};

use crate::modules::map::stores::map_actions::map_types::MapVenueKind;

/// read straight from the device and never waits for the server so the pin stands offline too
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapSelfLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_meters: f64,
    pub speed_meters_per_second: f64,
    pub course_degrees: f64,
    pub time_ms: i64,
    pub is_at_home: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_kind: Option<MapVenueKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stay_since_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_since_ms: Option<i64>,
}

impl MapSelfLocation {
    pub fn coordinate(&self) -> LatLng {
        LatLng::new(self.latitude, self.longitude)
    }

    pub fn has_course(&self) -> bool {
        self.course_degrees >= 0.0
    }
}

/// where the device is pointing, the cone under the reader's pin
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MapSelfHeading {
    pub degrees: f64,
    pub accuracy_degrees: f64,
}

pub struct MapSelfLocationFeedObserver {
    pub on_location: Box<dyn Fn(Option<MapSelfLocation>, &mut App)>,
    pub on_heading: Box<dyn Fn(Option<MapSelfHeading>, &mut App)>,
}

/// compass turns many times a second
/// the pin reads it here directly past every observable store so a turning device re-renders no view
pub struct MapSelfLocationFeed {
    location: Option<MapSelfLocation>,
    heading: Option<MapSelfHeading>,
    is_location_available: Option<bool>,
    observers: HashMap<Uuid, Rc<MapSelfLocationFeedObserver>>,
}

impl MapSelfLocationFeed {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            location: None,
            heading: None,
            is_location_available: None,
            observers: HashMap::new(),
        }
    }

    pub fn location(&self) -> Option<&MapSelfLocation> {
        self.location.as_ref()
    }

    pub fn heading(&self) -> Option<&MapSelfHeading> {
        self.heading.as_ref()
    }

    pub fn is_location_available(&self) -> Option<bool> {
        self.is_location_available
    }

    pub fn observe(&mut self, observer: MapSelfLocationFeedObserver) -> Uuid {
        let id = Uuid::new_v4();
        self.observers.insert(id, Rc::new(observer));
        id
    }

    pub fn cancel(&mut self, id: Uuid) {
        self.observers.remove(&id);
    }

    fn current_observers(&self) -> Vec<Rc<MapSelfLocationFeedObserver>> {
        self.observers.values().cloned().collect()
    }

    pub fn publish(&mut self, next: MapSelfLocation, cx: &mut Context<Self>) {
        let mut next = next;
        match self.is_location_available {
            Some(true) => next.unavailable_since_ms = None,
            Some(false) => {
                next.unavailable_since_ms = self
                    .location
                    .as_ref()
                    .and_then(|location| location.unavailable_since_ms)
                    .or(next.unavailable_since_ms)
                    .or_else(|| Some(RiseClock::now().ms()));
            }
            None => {}
        }
        let previous_time_ms = self.location.as_ref().map(|location| location.time_ms).unwrap_or(0);
        if self.location.as_ref() == Some(&next) || next.time_ms < previous_time_ms {
            return;
        }
        self.location = Some(next.clone());
        cx.notify();
        for observer in self.current_observers() {
            (observer.on_location)(Some(next.clone()), cx);
        }
    }

    pub fn set_location_available(&mut self, available: bool, cx: &mut Context<Self>) {
        if self.is_location_available == Some(available) {
            return;
        }
        self.is_location_available = Some(available);
        cx.notify();
        if let Some(location) = self.location.clone() {
            self.publish(location, cx);
        } else {
            for observer in self.current_observers() {
                (observer.on_location)(None, cx);
            }
        }
    }

    pub fn publish_heading(&mut self, next: Option<MapSelfHeading>, cx: &mut Context<Self>) {
        if next == self.heading {
            return;
        }
        self.heading = next;
        cx.notify();
        for observer in self.current_observers() {
            (observer.on_heading)(next, cx);
        }
    }

    /// logout or account switch, the next account never sees this one's spot
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.location = None;
        self.heading = None;
        self.is_location_available = None;
        cx.notify();
        for observer in self.current_observers() {
            (observer.on_location)(None, cx);
            (observer.on_heading)(None, cx);
        }
    }
}
