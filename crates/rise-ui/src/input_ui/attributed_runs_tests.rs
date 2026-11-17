use gpui::hsla;

use super::*;

#[derive(Clone, PartialEq, Debug)]
struct Mark(&'static str);

fn plain() -> RunAttributes {
    RunAttributes::default()
}

fn red() -> RunAttributes {
    RunAttributes::new(AttributeContainer {
        foreground_color: Some(hsla(0.0, 1.0, 0.5, 1.0)),
        ..AttributeContainer::default()
    })
}

fn marked(name: &'static str) -> RunAttributes {
    RunAttributes::tagged(AttributeContainer::default(), Mark(name))
}

fn lengths(runs: &AttributedRuns) -> Vec<usize> {
    runs.runs().map(|(range, _)| range.len()).collect()
}

#[test]
fn an_empty_text_has_no_run() {
    let runs = AttributedRuns::new(0, plain());
    assert!(runs.is_empty());
    assert_eq!(runs.len(), 0);
}

#[test]
fn equal_neighbours_merge_and_empty_pieces_vanish() {
    let runs = AttributedRuns::from_pieces([(2, plain()), (0, red()), (3, plain()), (1, red())]);
    assert_eq!(lengths(&runs), vec![5, 1]);
}

#[test]
fn typing_inside_a_run_grows_it() {
    let mut runs = AttributedRuns::from_pieces([(3, plain()), (3, red())]);
    runs.splice(4..4, 2, red());
    assert_eq!(lengths(&runs), vec![3, 5]);
}

#[test]
fn a_replacement_across_runs_cuts_both_edges() {
    let mut runs = AttributedRuns::from_pieces([(4, plain()), (4, red()), (4, marked("bold"))]);
    runs.splice(2..10, 1, marked("link"));
    assert_eq!(lengths(&runs), vec![2, 1, 2]);
    assert_eq!(runs.len(), 5);
    let tags: Vec<Option<&Mark>> = runs.runs().map(|(_, run)| run.tag::<Mark>()).collect();
    assert_eq!(tags, vec![None, Some(&Mark("link")), Some(&Mark("bold"))]);
}

#[test]
fn a_deletion_drops_the_runs_it_covers() {
    let mut runs = AttributedRuns::from_pieces([(2, plain()), (2, red()), (2, plain())]);
    runs.splice(2..4, 0, red());
    assert_eq!(lengths(&runs), vec![4]);
}

#[test]
fn tags_of_different_values_do_not_merge() {
    let runs = AttributedRuns::from_pieces([(1, marked("a")), (1, marked("b")), (1, marked("b"))]);
    assert_eq!(lengths(&runs), vec![1, 2]);
}

#[test]
fn a_tag_reads_back_only_as_its_own_type() {
    let attributes = marked("code");
    assert_eq!(attributes.tag::<Mark>(), Some(&Mark("code")));
    assert_eq!(attributes.tag::<u32>(), None);
    assert_ne!(RunTag::new(1_u32), RunTag::new(1_u64));
}

#[test]
fn typed_text_inherits_like_uitextview() {
    let runs = AttributedRuns::from_pieces([(2, plain()), (2, red())]);
    assert_eq!(runs.inherited_for(&(2..2)), Some(plain()));
    assert_eq!(runs.inherited_for(&(3..3)), Some(red()));
    assert_eq!(runs.inherited_for(&(2..4)), Some(red()));
    assert_eq!(runs.inherited_for(&(0..0)), Some(plain()));
    assert_eq!(runs.inherited_for(&(4..4)), Some(red()));
    assert_eq!(AttributedRuns::default().inherited_for(&(0..0)), None);
}

#[test]
fn update_restyles_only_the_range() {
    let mut runs = AttributedRuns::new(6, plain());
    runs.set(2..4, &red());
    assert_eq!(lengths(&runs), vec![2, 2, 2]);
    runs.update(0..6, |run| run.attributes.foreground_color = None);
    assert_eq!(lengths(&runs), vec![6]);
}
