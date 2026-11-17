use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, IntoElement, ParentElement, Styled, Window, div, px};

use crate::list_ui::list_ui::ListUi;
use crate::swiftui::animated::{Animated, MotionTime};
use crate::swiftui::measure::measure;
use crate::swiftui::motion::MotionSpec;

/// a row the list holds now
/// a leaving row is still drawn but not in the model, keyboard targets and row actions skip it
#[derive(Clone)]
pub struct ListShownRow<R> {
    pub id: String,
    pub row: R,
    pub is_leaving: bool,
}

struct RowMotion {
    progress: Animated<f32>,
    height: Option<f32>,
    is_leaving: bool,
}

type RowMotions = Rc<RefCell<HashMap<String, RowMotion>>>;

/// keep it next to the ListUi it drives with no other splice calls on that list
pub struct ListRowTransitions<R> {
    shown: Rc<Vec<ListShownRow<R>>>,
    motions: RowMotions,
}

impl<R> Default for ListRowTransitions<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R> ListRowTransitions<R> {
    pub fn new() -> Self {
        Self {
            shown: Rc::new(Vec::new()),
            motions: Rc::new(RefCell::new(HashMap::new())),
        }
    }

    pub fn is_animating(&self) -> bool {
        !self.motions.borrow().is_empty()
    }
}

impl<R: Clone> ListRowTransitions<R> {
    /// brings the list to next, a list of stable row id plus row
    /// call from render before the list element is built, spec is the animation of the change
    /// none lands new and removed rows at once
    pub fn sync(
        &mut self,
        list: &ListUi,
        next: Vec<(String, R)>,
        spec: Option<MotionSpec>,
        now: MotionTime,
        window: &Window,
        cx: &App,
    ) -> ListRowsFrame<R> {
        let settled = {
            let mut motions = self.motions.borrow_mut();
            start_motions(&self.shown, &next, list, &mut motions, spec, now, cx);
            settle(&mut motions, now)
        };
        let shown = {
            let motions = self.motions.borrow();
            merged_rows(&self.shown, next, |id| motions.get(id).is_some_and(|motion| motion.is_leaving))
        };
        splice_changed(list, &self.shown, &shown);

        let motions = self.motions.borrow();
        if !motions.is_empty() || !settled.is_empty() {
            for (index, row) in shown.iter().enumerate() {
                if motions.contains_key(&row.id) || settled.contains(&row.id) {
                    list.state().remeasure_items(index..index + 1);
                }
            }
        }
        if !motions.is_empty() {
            window.request_animation_frame();
        }
        drop(motions);

        self.shown = Rc::new(shown);
        ListRowsFrame {
            rows: Rc::clone(&self.shown),
            motions: Rc::clone(&self.motions),
            now,
        }
    }
}

/// one frame of the list, what the ListUi element closure reads
pub struct ListRowsFrame<R> {
    rows: Rc<Vec<ListShownRow<R>>>,
    motions: RowMotions,
    now: MotionTime,
}

impl<R> ListRowsFrame<R> {
    /// rows in list order, a list index is an index into this slice
    pub fn rows(&self) -> &[ListShownRow<R>] {
        &self.rows
    }

    /// content as the list has to draw it this frame, a row at rest comes back untouched
    pub fn element(&self, id: &str, content: AnyElement) -> AnyElement {
        let (progress, height) = {
            let motions = self.motions.borrow();
            let Some(motion) = motions.get(id) else {
                return content;
            };
            (motion.progress.value(self.now).clamp(0.0, 1.0), motion.height)
        };
        let motions = Rc::clone(&self.motions);
        let measured_id = id.to_owned();
        div()
            .w_full()
            .h(px(height.unwrap_or(0.0) * progress))
            .opacity(if height.is_some() { progress } else { 0.0 })
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex_none()
                    .child(content)
                    .child(measure(ElementId::Name(format!("list-row-transition-{id}").into()), move |bounds, _, _| {
                        if let Some(motion) = motions.borrow_mut().get_mut(&measured_id) {
                            motion.height = Some(f32::from(bounds.size.height));
                        }
                    })),
            )
            .into_any_element()
    }
}

fn start_motions<R>(
    shown: &[ListShownRow<R>],
    next: &[(String, R)],
    list: &ListUi,
    motions: &mut HashMap<String, RowMotion>,
    spec: Option<MotionSpec>,
    now: MotionTime,
    cx: &App,
) {
    let next_ids: HashSet<&str> = next.iter().map(|(id, _)| id.as_str()).collect();
    let shown_ids: HashSet<&str> = shown.iter().map(|row| row.id.as_str()).collect();

    for (index, row) in shown.iter().enumerate() {
        if next_ids.contains(row.id.as_str()) {
            continue;
        }
        if let Some(motion) = motions.get_mut(&row.id) {
            motion.is_leaving = true;
            motion.progress.set_in(0.0, spec, now, cx);
            continue;
        }
        let Some(bounds) = list.state().bounds_for_item(index) else {
            continue;
        };
        let mut progress = Animated::new(1.0_f32);
        progress.set_in(0.0, spec, now, cx);
        motions.insert(
            row.id.clone(),
            RowMotion {
                progress,
                height: Some(f32::from(bounds.size.height)),
                is_leaving: true,
            },
        );
    }

    for (id, _) in next {
        if let Some(motion) = motions.get_mut(id) {
            if motion.is_leaving {
                motion.is_leaving = false;
                motion.progress.set_in(1.0, spec, now, cx);
            }
        } else if spec.is_some() && !shown_ids.contains(id.as_str()) {
            let mut progress = Animated::new(0.0_f32);
            progress.set_in(1.0, spec, now, cx);
            motions.insert(
                id.clone(),
                RowMotion {
                    progress,
                    height: None,
                    is_leaving: false,
                },
            );
        }
    }
}

/// drops the motions that ended and names their rows
fn settle(motions: &mut HashMap<String, RowMotion>, now: MotionTime) -> HashSet<String> {
    let settled: HashSet<String> =
        motions.iter().filter(|(_, motion)| !motion.progress.is_running(now)).map(|(id, _)| id.clone()).collect();
    motions.retain(|id, _| !settled.contains(id));
    settled
}

/// next with each still leaving row put back behind the row it followed
pub(crate) fn merged_rows<R: Clone>(
    shown: &[ListShownRow<R>],
    next: Vec<(String, R)>,
    still_leaving: impl Fn(&str) -> bool,
) -> Vec<ListShownRow<R>> {
    let next_ids: HashSet<String> = next.iter().map(|(id, _)| id.clone()).collect();
    let mut merged: Vec<ListShownRow<R>> =
        next.into_iter().map(|(id, row)| ListShownRow { id, row, is_leaving: false }).collect();
    let mut previous: Option<&str> = None;
    for row in shown {
        if next_ids.contains(&row.id) {
            previous = Some(row.id.as_str());
            continue;
        }
        if !still_leaving(&row.id) {
            continue;
        }
        let at = previous
            .and_then(|previous| merged.iter().position(|candidate| candidate.id == previous))
            .map_or(0, |position| position + 1);
        merged.insert(
            at,
            ListShownRow {
                id: row.id.clone(),
                row: row.row.clone(),
                is_leaving: true,
            },
        );
        previous = Some(row.id.as_str());
    }
    merged
}

fn splice_changed<R>(list: &ListUi, old: &[ListShownRow<R>], new: &[ListShownRow<R>]) {
    let prefix = old.iter().zip(new).take_while(|(old, new)| old.id == new.id).count();
    if prefix == old.len() && prefix == new.len() {
        return;
    }
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old.iter().rev().zip(new.iter().rev()).take(max_suffix).take_while(|(old, new)| old.id == new.id).count();
    let old_middle = &old[prefix..old.len() - suffix];
    let new_middle = &new[prefix..new.len() - suffix];
    let mut new_positions: HashMap<&str, usize> = HashMap::with_capacity(new_middle.len());
    for (position, row) in new_middle.iter().enumerate() {
        new_positions.entry(row.id.as_str()).or_insert(position);
    }
    let shared = old_middle
        .iter()
        .enumerate()
        .find_map(|(old_at, row)| new_positions.get(row.id.as_str()).map(|&new_at| (old_at, new_at)));
    let Some((old_at, new_at)) = shared else {
        list.state().splice(prefix..prefix + old_middle.len(), new_middle.len());
        return;
    };
    let run = old_middle[old_at..].iter().zip(&new_middle[new_at..]).take_while(|(old, new)| old.id == new.id).count();
    let tail_start = prefix + old_at + run;
    let tail_end = prefix + old_middle.len();
    let tail_count = new_middle.len() - new_at - run;
    if tail_start != tail_end || tail_count != 0 {
        list.state().splice(tail_start..tail_end, tail_count);
    }
    if old_at != 0 || new_at != 0 {
        list.state().splice(prefix..prefix + old_at, new_at);
    }
}
