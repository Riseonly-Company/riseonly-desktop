use gpui::Pixels;
use rise_theme::ListMetrics;

/// Which end of the list is asking for another page.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaginationEdge {
    /// Older items, below. The common one.
    Bottom,
    /// Newer items, above — a chat history, or a feed scrolled back into.
    Top,
}

/// What the list looks like right now, in pixels rather than row indices.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScrollProbe {
    pub viewport_height: Pixels,
    pub distance_to_top: Pixels,
    pub distance_to_bottom: Pixels,
    /// Points per second, positive downwards. Zero when nothing measures it.
    pub velocity: f32,
}

/// What each edge is allowed to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct EdgeState {
    pub has_more_top: bool,
    pub has_more_bottom: bool,
    /// A request is already out for this edge; without it a slow response means
    /// one request per frame for as long as the edge stays in range.
    pub top_in_flight: bool,
    pub bottom_in_flight: bool,
}

/// How far from an edge a page is asked for: a viewport term plus a velocity
/// term, clamped at both ends.
pub fn trigger_distance(metrics: &ListMetrics, probe: &ScrollProbe) -> Pixels {
    let velocity = if probe.velocity.is_finite() {
        probe.velocity.abs()
    } else {
        0.0
    };

    let viewport = f32::from(probe.viewport_height).max(0.0) * ListMetrics::LEAD_VIEWPORTS;
    let lead = velocity * ListMetrics::LEAD_SECONDS;

    gpui::px(viewport + lead).clamp(
        metrics.pagination_min_distance,
        metrics.pagination_max_distance,
    )
}

/// The edge to load, if any.
///
/// An edge is armed only when the list is moving towards it, or when it is
/// already within the floor distance.
pub fn evaluate(
    metrics: &ListMetrics,
    probe: &ScrollProbe,
    edges: &EdgeState,
) -> Option<PaginationEdge> {
    let threshold = trigger_distance(metrics, probe);
    let floor = metrics.pagination_min_distance;
    let velocity = if probe.velocity.is_finite() {
        probe.velocity
    } else {
        0.0
    };

    let approaching_bottom = velocity >= 0.0;
    let approaching_top = velocity <= 0.0;

    let bottom_ready = edges.has_more_bottom
        && !edges.bottom_in_flight
        && (probe.distance_to_bottom <= floor
            || (approaching_bottom && probe.distance_to_bottom <= threshold));

    let top_ready = edges.has_more_top
        && !edges.top_in_flight
        && (probe.distance_to_top <= floor
            || (approaching_top && probe.distance_to_top <= threshold));

    // The bottom wins a tie: the user is more likely to be reading forwards.
    match (bottom_ready, top_ready) {
        (true, _) => Some(PaginationEdge::Bottom),
        (false, true) => Some(PaginationEdge::Top),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    fn metrics() -> ListMetrics {
        ListMetrics::default()
    }

    fn probe(distance_to_bottom: f32, velocity: f32) -> ScrollProbe {
        ScrollProbe {
            viewport_height: px(800.0),
            distance_to_top: px(10_000.0),
            distance_to_bottom: px(distance_to_bottom),
            velocity,
        }
    }

    fn both_ends() -> EdgeState {
        EdgeState {
            has_more_top: true,
            has_more_bottom: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_threshold_is_a_viewport_at_rest() {
        let at_rest = probe(5_000.0, 0.0);
        assert_eq!(trigger_distance(&metrics(), &at_rest), px(800.0));
    }

    #[test]
    fn a_fling_asks_earlier_than_a_drag() {
        let slow = trigger_distance(&metrics(), &probe(5_000.0, 200.0));
        let fast = trigger_distance(&metrics(), &probe(5_000.0, 4_000.0));

        assert!(
            fast > slow,
            "a fling covers a screen in the time a request takes, so it has to ask sooner"
        );
    }

    #[test]
    fn the_threshold_is_clamped_at_both_ends() {
        let tiny = ScrollProbe {
            viewport_height: px(40.0),
            ..probe(5_000.0, 0.0)
        };
        assert_eq!(
            trigger_distance(&metrics(), &tiny),
            metrics().pagination_min_distance,
            "a short window must not ask so late that the user reaches the end first"
        );

        let flung = probe(5_000.0, 100_000.0);
        assert_eq!(
            trigger_distance(&metrics(), &flung),
            metrics().pagination_max_distance,
            "a fling must not pull the whole timeline it is about to scroll past"
        );
    }

    #[test]
    fn a_broken_velocity_falls_back_to_the_viewport_term() {
        assert_eq!(
            trigger_distance(&metrics(), &probe(5_000.0, f32::NAN)),
            px(800.0)
        );
        assert_eq!(
            trigger_distance(&metrics(), &probe(5_000.0, f32::INFINITY)),
            px(800.0)
        );
    }

    #[test]
    fn nothing_is_asked_for_while_the_end_is_still_far_away() {
        assert_eq!(
            evaluate(&metrics(), &probe(5_000.0, 300.0), &both_ends()),
            None
        );
    }

    #[test]
    fn the_bottom_is_asked_for_while_the_list_is_still_moving() {
        assert_eq!(
            evaluate(&metrics(), &probe(900.0, 2_000.0), &both_ends()),
            Some(PaginationEdge::Bottom)
        );
    }

    #[test]
    fn an_edge_already_in_flight_is_not_asked_for_again() {
        let edges = EdgeState {
            bottom_in_flight: true,
            ..both_ends()
        };
        assert_eq!(evaluate(&metrics(), &probe(100.0, 2_000.0), &edges), None);
    }

    #[test]
    fn an_edge_with_nothing_behind_it_is_never_asked_for() {
        let edges = EdgeState {
            has_more_bottom: false,
            has_more_top: false,
            ..Default::default()
        };
        assert_eq!(evaluate(&metrics(), &probe(0.0, 2_000.0), &edges), None);
    }

    #[test]
    fn scrolling_up_does_not_fetch_the_bottom_it_is_moving_away_from() {
        let leaving = ScrollProbe {
            distance_to_top: px(10_000.0),
            ..probe(700.0, -3_000.0)
        };
        assert_eq!(
            evaluate(&metrics(), &leaving, &both_ends()),
            None,
            "the bottom is within a fling's reach, but the fling is going the other way"
        );
    }

    #[test]
    fn an_edge_already_underfoot_is_fetched_whichever_way_the_list_is_moving() {
        // The list that opened one page short of a full viewport.
        let underfoot = probe(50.0, -3_000.0);
        assert_eq!(
            evaluate(&metrics(), &underfoot, &both_ends()),
            Some(PaginationEdge::Bottom)
        );
    }

    #[test]
    fn scrolling_up_fetches_the_top() {
        let rising = ScrollProbe {
            distance_to_top: px(600.0),
            distance_to_bottom: px(50_000.0),
            velocity: -2_000.0,
            viewport_height: px(800.0),
        };
        assert_eq!(
            evaluate(&metrics(), &rising, &both_ends()),
            Some(PaginationEdge::Top)
        );
    }

    #[test]
    fn a_list_short_enough_to_touch_both_edges_reads_forwards_first() {
        let short = ScrollProbe {
            viewport_height: px(800.0),
            distance_to_top: px(0.0),
            distance_to_bottom: px(0.0),
            velocity: 0.0,
        };
        assert_eq!(
            evaluate(&metrics(), &short, &both_ends()),
            Some(PaginationEdge::Bottom)
        );
    }

    #[test]
    fn a_failed_page_can_be_asked_for_again_as_soon_as_it_is_no_longer_in_flight() {
        let at_edge = probe(100.0, 500.0);

        let during = EdgeState {
            bottom_in_flight: true,
            ..both_ends()
        };
        assert_eq!(evaluate(&metrics(), &at_edge, &during), None);

        // The repository clears the flag on failure and keeps `has_more`.
        assert_eq!(
            evaluate(&metrics(), &at_edge, &both_ends()),
            Some(PaginationEdge::Bottom)
        );
    }
}
